//! The booking flow as a kit: kitewell:booking (wit/booking.wit). Each
//! slot's functions are their own interface with their own grant, so
//! policy.csv decides which app changes which slot, and AppOS checks it
//! before any of this runs. The functions write at once; each write is
//! harmless to repeat. The platform's own pages are in pages.rs.

use appos_shell::kit::{Grant, Kit, KitLinker, KitRoute};

use crate::catalog::{self, State};
use crate::store::{Store, looks_like_email};
use crate::types::{Billing, Customer, Draft, Member, PaymentTerms, Plan, Slot};

const FLOW: &str = "kitewell:booking/flow@0.1.0";
const CHOOSE: &str = "kitewell:booking/choose@0.1.0";
const CONFIGURE: &str = "kitewell:booking/configure@0.1.0";
const TERMS: &str = "kitewell:booking/terms@0.1.0";
const SETUP: &str = "kitewell:booking/setup@0.1.0";

/// The grant for the platform's own page at /book: review and book. It
/// allows no function.
pub const PAGES: &str = "booking-book";

/// The slots apps own, and the path of each one's page. A slot not here
/// (review) is the platform's own page.
pub type Slots = &'static [(Slot, &'static str)];

#[derive(Clone)]
pub struct Booking {
    pub(crate) store: Store,
    slots: Slots,
}

impl Booking {
    pub fn new(store: Store, slots: Slots) -> Booking {
        Booking { store, slots }
    }

    /// The draft as apps see it, with today's quote.
    pub(crate) fn view(&self, id: &str, state: State) -> rusqlite::Result<Draft> {
        let offer = match &state.offer {
            Some(code) => self.store.offer(code)?,
            None => None,
        };
        let quote = catalog::quote(&state, offer.as_ref(), &today());
        Ok(Draft {
            id: id.to_string(),
            package: state.package,
            plan: state.plan,
            users: state.users,
            add_ons: state.add_ons,
            template: state.template,
            billing: state.billing,
            years: state.years,
            payment_terms: state.payment_terms,
            offer: state.offer,
            quote,
        })
    }

    pub(crate) fn get(&self, id: &str) -> rusqlite::Result<Option<Draft>> {
        self.store
            .draft(id)?
            .map(|state| self.view(id, state))
            .transpose()
    }

    /// Changes a draft that isn't booked yet, if `change` allows it, and
    /// answers with the new draft or the reason in words.
    fn change(
        &self,
        id: &str,
        change: impl FnOnce(&mut State) -> Result<(), String>,
    ) -> rusqlite::Result<Result<Draft, String>> {
        let Some(mut state) = self.store.draft(id)? else {
            return Ok(Err(
                "This booking isn't open any more. Please start again.".into()
            ));
        };
        if self.store.booking(id)?.is_some() {
            return Ok(Err("This booking is done.".into()));
        }
        if let Err(why) = change(&mut state) {
            return Ok(Err(why));
        }
        self.store.save(id, &state)?;
        self.view(id, state).map(Ok)
    }

    pub(crate) fn with_plan(
        &self,
        id: &str,
        plan: Plan,
        users: u32,
        package: &str,
    ) -> rusqlite::Result<Result<Draft, String>> {
        self.change(id, |s| {
            catalog::check_plan(plan, users)?;
            let package = package.trim();
            s.package = if package.is_empty() {
                format!("{plan:?}")
            } else {
                package.to_string()
            };
            (s.plan, s.users) = (Some(plan), users);
            // The offer's terms go with it.
            if s.offer.take().is_some() {
                let d = State::default();
                (s.billing, s.years, s.payment_terms) = (d.billing, d.years, d.payment_terms);
            }
            Ok(())
        })
    }

    pub(crate) fn with_offer(
        &self,
        id: &str,
        code: &str,
    ) -> rusqlite::Result<Result<Draft, String>> {
        let Some(offer) = self.store.offer(code)? else {
            return Ok(Err(format!("There is no offer {}", code.trim())));
        };
        if let Some(why) = offer.expired(&today()) {
            return Ok(Err(why));
        }
        self.change(id, |s| {
            s.package = offer.package.clone();
            (s.plan, s.users) = (Some(offer.plan), offer.users);
            (s.billing, s.years, s.payment_terms) =
                (offer.billing, offer.years, offer.payment_terms);
            s.offer = Some(offer.code.clone());
            Ok(())
        })
    }

    pub(crate) fn with_add_on(
        &self,
        id: &str,
        add_on: &str,
        on: bool,
    ) -> rusqlite::Result<Result<Draft, String>> {
        self.change(id, |s| {
            if !catalog::catalog().add_ons.iter().any(|a| a.id == add_on) {
                return Err(format!("There is no add-on {add_on}"));
            }
            s.add_ons.retain(|a| a != add_on);
            if on {
                s.add_ons.push(add_on.to_string());
            }
            Ok(())
        })
    }

    pub(crate) fn with_template(
        &self,
        id: &str,
        template: Option<&str>,
    ) -> rusqlite::Result<Result<Draft, String>> {
        self.change(id, |s| {
            if let Some(t) = template
                && catalog::template_name(t).is_none()
            {
                return Err(format!("There is no template {t}"));
            }
            s.template = template.map(str::to_string);
            Ok(())
        })
    }

    pub(crate) fn with_terms(
        &self,
        id: &str,
        billing: Billing,
        years: u8,
        payment_terms: PaymentTerms,
    ) -> rusqlite::Result<Result<Draft, String>> {
        let offer = match self.store.draft(id)?.and_then(|s| s.offer) {
            Some(code) => self.store.offer(&code)?,
            None => None,
        };
        self.change(id, |s| {
            catalog::check_terms(offer.as_ref(), billing, years, payment_terms)?;
            (s.billing, s.years, s.payment_terms) = (billing, years, payment_terms);
            Ok(())
        })
    }

    /// The page of a slot an app owns; none for review, the platform's own.
    pub(crate) fn app_page(&self, slot: Slot) -> Option<&'static str> {
        self.slots
            .iter()
            .find(|(s, _)| *s == slot)
            .map(|(_, path)| *path)
    }

    /// Where a slot's page is for a draft: the app's, or the platform's
    /// own for review.
    pub(crate) fn link(&self, id: &str, to: Slot) -> String {
        match self.app_page(to) {
            Some(path) => format!("{path}?draft={id}"),
            None => format!("/book/{id}"),
        }
    }

    pub(crate) fn next(&self, id: &str, after: Slot) -> String {
        let next = match after {
            Slot::Choose => Slot::Configure,
            Slot::Configure => Slot::Terms,
            Slot::Terms => Slot::Review,
            Slot::Review | Slot::Setup => Slot::Setup,
        };
        self.link(id, next)
    }

    /// Books a draft as it is now, for the customer, who agreed to
    /// `consent`: once per draft. The reasons in words when it can't be
    /// booked.
    pub(crate) fn book(
        &self,
        id: &str,
        customer: &Customer,
        consent: &str,
    ) -> anyhow::Result<Result<(), Vec<String>>> {
        if self.store.booking(id)?.is_some() {
            return Ok(Ok(()));
        }
        let Some(state) = self.store.draft(id)? else {
            return Ok(Err(vec!["This booking isn't open any more".into()]));
        };
        let draft = self.view(id, state.clone())?;
        let mut problems = draft.quote.problems.clone();
        if customer.company.trim().is_empty() {
            problems.push("The company's name is missing".into());
        }
        if customer.name.trim().is_empty() {
            problems.push("Your name is missing".into());
        }
        if !looks_like_email(&customer.email) {
            problems.push("The email address doesn't look right".into());
        }
        if consent.trim().is_empty() {
            problems.push("The terms weren't agreed to".into());
        }
        if !problems.is_empty() {
            return Ok(Err(problems));
        }
        let customer = Customer {
            company: customer.company.trim().into(),
            name: customer.name.trim().into(),
            email: customer.email.trim().into(),
        };
        self.store
            .book(id, &customer, consent, &state, &draft.quote, &now())?;
        Ok(Ok(()))
    }

    pub(crate) fn add_member(
        &self,
        booking: &str,
        name: &str,
        email: &str,
    ) -> anyhow::Result<Result<Member, String>> {
        self.store.add_member(booking, name, email, &now())
    }
}

pub(crate) fn now() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn today() -> String {
    chrono::Utc::now().format("%Y-%m-%d").to_string()
}

/// A function's answer: its value, or the store's error as the call's.
macro_rules! answer {
    ($value:expr) => {{
        let value = $value.map_err(|e| wasmtime::format_err!("{e:#}"));
        Box::pin(async move { Ok((value?,)) })
    }};
}

impl Kit for Booking {
    fn wit(&self) -> Vec<(String, String)> {
        vec![(
            "booking.wit".into(),
            include_str!("../wit/booking.wit").into(),
        )]
    }

    fn grants(&self) -> Vec<Grant> {
        let grant = |name: &str, allows: &str, interface: &str, functions: &[&str], cost: u32| {
            let functions: Vec<String> = functions
                .iter()
                .map(|f| format!("kitewell:booking/{interface}#{f}"))
                .collect();
            Grant::new(
                name,
                allows,
                &functions.iter().map(String::as_str).collect::<Vec<_>>(),
                cost,
            )
        };
        vec![
            grant(
                "booking-read",
                "Read the catalog, drafts and bookings, and where each slot is: flow::catalog, get, link, next, booked",
                "flow",
                &["catalog", "get", "link", "next", "booked"],
                1,
            ),
            grant(
                "booking-choose",
                "Start a draft and set its package, or an offer: choose::start, with-plan, with-offer",
                "choose",
                &["start", "with-plan", "with-offer"],
                2,
            ),
            grant(
                "booking-configure",
                "Set a draft's add-ons and template: configure::with-add-on, with-template",
                "configure",
                &["with-add-on", "with-template"],
                2,
            ),
            grant(
                "booking-terms",
                "Set a draft's billing, contract length and payment terms: terms::with-terms",
                "terms",
                &["with-terms"],
                2,
            ),
            grant(
                "booking-setup",
                "Add members to a booked workspace: setup::add-member",
                "setup",
                &["add-member"],
                2,
            ),
            Grant::new(
                PAGES,
                "Book on Kitewell's own page, /book: review and book",
                &[],
                1,
            ),
        ]
    }

    fn link(&self, linker: &mut KitLinker<'_>) -> anyhow::Result<()> {
        linker.func(FLOW, "catalog", move |_, ()| {
            answer!(Ok::<_, anyhow::Error>(catalog::catalog()))
        })?;
        let b = self.clone();
        linker.func(FLOW, "get", move |_, (id,): (String,)| answer!(b.get(&id)))?;
        let b = self.clone();
        linker.func(FLOW, "link", move |_, (id, to): (String, Slot)| {
            answer!(Ok::<_, anyhow::Error>(b.link(&id, to)))
        })?;
        let b = self.clone();
        linker.func(FLOW, "next", move |_, (id, after): (String, Slot)| {
            answer!(Ok::<_, anyhow::Error>(b.next(&id, after)))
        })?;
        let b = self.clone();
        linker.func(FLOW, "booked", move |_, (id,): (String,)| {
            answer!(b.store.booking(&id))
        })?;

        let b = self.clone();
        linker.func(CHOOSE, "start", move |_, ()| {
            answer!(
                b.store
                    .start()
                    .and_then(|(id, state)| Ok(b.view(&id, state)?))
            )
        })?;
        let b = self.clone();
        linker.func(
            CHOOSE,
            "with-plan",
            move |_, (id, plan, users, package): (String, Plan, u32, String)| {
                answer!(b.with_plan(&id, plan, users, &package))
            },
        )?;
        let b = self.clone();
        linker.func(
            CHOOSE,
            "with-offer",
            move |_, (id, code): (String, String)| answer!(b.with_offer(&id, &code)),
        )?;

        let b = self.clone();
        linker.func(
            CONFIGURE,
            "with-add-on",
            move |_, (id, add_on, on): (String, String, bool)| {
                answer!(b.with_add_on(&id, &add_on, on))
            },
        )?;
        let b = self.clone();
        linker.func(
            CONFIGURE,
            "with-template",
            move |_, (id, template): (String, Option<String>)| {
                answer!(b.with_template(&id, template.as_deref()))
            },
        )?;

        let b = self.clone();
        linker.func(
            TERMS,
            "with-terms",
            move |_, (id, billing, years, terms): (String, Billing, u8, PaymentTerms)| {
                answer!(b.with_terms(&id, billing, years, terms))
            },
        )?;

        let b = self.clone();
        linker.func(
            SETUP,
            "add-member",
            move |_, (booking, name, email): (String, String, String)| {
                answer!(b.add_member(&booking, &name, &email))
            },
        )
    }

    fn routes(&self) -> Vec<KitRoute> {
        crate::pages::routes(self)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SLOTS: Slots = &[
        (Slot::Choose, "/packages"),
        (Slot::Configure, "/workspace/configure"),
        (Slot::Terms, "/packages/terms"),
        (Slot::Setup, "/workspace/setup"),
    ];

    pub(crate) fn booking() -> Booking {
        Booking::new(Store::open(":memory:").unwrap(), SLOTS)
    }

    pub(crate) fn customer() -> Customer {
        Customer {
            company: "Acme Analytics".into(),
            name: "Ann Lee".into(),
            email: "ann@acme.example".into(),
        }
    }

    #[test]
    fn a_booking_through_every_slot() {
        let b = booking();
        let (id, _) = b.store.start().unwrap();
        assert_eq!(
            b.with_plan(&id, Plan::Starter, 11, "Starter").unwrap(),
            Err("Starter is for up to 10 users".into())
        );
        let d = b.with_plan(&id, Plan::Team, 25, "Team").unwrap().unwrap();
        assert_eq!(d.quote.per_year_cents, 725_000);
        let d = b.with_add_on(&id, catalog::AI, true).unwrap().unwrap();
        assert_eq!(d.quote.per_year_cents, 725_000 + 200_000);
        assert!(b.with_template(&id, Some("nope")).unwrap().is_err());
        b.with_template(&id, Some("plan-a-quarter"))
            .unwrap()
            .unwrap();
        assert_eq!(
            b.with_terms(&id, Billing::Monthly, 2, PaymentTerms::Net30)
                .unwrap(),
            Err("Monthly billing is for 1 year only".into())
        );
        b.with_terms(&id, Billing::Yearly, 2, PaymentTerms::Net30)
            .unwrap()
            .unwrap();

        b.book(&id, &customer(), "I agree.").unwrap().unwrap();
        let booked = b.store.booking(&id).unwrap().unwrap();
        assert_eq!(booked.project.as_deref(), Some("Plan a quarter"));
        assert_eq!(booked.quote.contract_cents, 2 * 925_000);
        assert_eq!(
            b.with_plan(&id, Plan::Team, 30, "Team").unwrap(),
            Err("This booking is done.".into())
        );
        assert_eq!(
            b.add_member(&id, "Tom", "tom@acme.example")
                .unwrap()
                .map(|m| m.name),
            Ok("Tom".into())
        );
    }

    #[test]
    fn each_slot_links_to_its_page() {
        let b = booking();
        assert_eq!(b.next("d1", Slot::Choose), "/workspace/configure?draft=d1");
        assert_eq!(b.next("d1", Slot::Terms), "/book/d1");
        assert_eq!(b.next("d1", Slot::Review), "/workspace/setup?draft=d1");
        assert_eq!(b.link("d1", Slot::Terms), "/packages/terms?draft=d1");
        assert_eq!(b.app_page(Slot::Review), None);
    }

    #[test]
    fn an_offer_sets_the_package_and_its_terms() {
        let b = booking();
        let (id, _) = b.store.start().unwrap();
        assert_eq!(
            b.with_offer(&id, "KW-POLAR-2025").unwrap(),
            Err("This offer expired on 31 December 2025".into())
        );
        let d = b.with_offer(&id, " kw-nordlicht-2026 ").unwrap().unwrap();
        assert_eq!(
            (d.package.as_str(), d.users, d.billing),
            ("Agency package", 40, Billing::Quarterly)
        );
        assert_eq!(
            b.with_terms(&id, Billing::Yearly, 1, PaymentTerms::Net30)
                .unwrap(),
            Err("These terms come with your offer KW-NORDLICHT-2026".into())
        );
        let d = b.with_plan(&id, Plan::Team, 40, "Team").unwrap().unwrap();
        assert_eq!(
            (d.offer, d.billing, d.payment_terms),
            (None, Billing::Yearly, PaymentTerms::Net30),
            "the offer's terms went with it"
        );
    }

    #[test]
    fn what_cant_be_booked_says_why_and_books_nothing() {
        let b = booking();
        let (id, _) = b.store.start().unwrap();
        let problems = b
            .book(
                &id,
                &Customer {
                    email: "ann".into(),
                    ..customer()
                },
                "I agree.",
            )
            .unwrap();
        assert_eq!(
            problems,
            Err(vec![
                "Choose a package".to_string(),
                "The email address doesn't look right".to_string()
            ])
        );
        assert!(b.store.booking(&id).unwrap().is_none());
    }

    #[test]
    fn the_platform_takes_the_kit() {
        let contract = appos_shell::kit::Kits::builtin()
            .with(booking())
            .unwrap()
            .contract();
        assert!(
            contract["appos.wit"].contains("    import kitewell:booking/flow@0.1.0;\n"),
            "{}",
            contract["appos.wit"]
        );
    }
}
