//! The pages and what they show. Every text is formatted here, so the
//! templates only place it; each page is a template struct.

use askama::Template;

use super::domain::{
    self, Billing, DiscountRequest, FormErrors, POLICY, Payment, Plan, Preview, Status, TermsInput,
};
use super::money::{dollars, dollars_short, group};
use super::people::{self, PEOPLE, Person, Rep, Role};
use super::stats::{self, QuarterStats, Tally};
use super::time;

// ---- Shared parts --------------------------------------------------------

pub struct Choice {
    pub value: String,
    pub label: String,
    pub selected: bool,
}

pub struct NavLink {
    pub label: &'static str,
    pub url: String,
    pub active: bool,
    pub badge: String,
    pub hot: bool,
}

/// The bar on every page: who you are, where you can go.
pub struct Frame {
    pub base: String,
    /// "/discounts/as/daniel": every link of this person starts here.
    pub home: String,
    /// The id of the person looking.
    pub me: String,
    pub people: Vec<Choice>,
    pub nav: Vec<NavLink>,
    /// The page to stay on when switching person: "r/7", "all", "new".
    pub here: String,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Home,
    New,
    All,
    Other,
}

pub fn frame(
    base: &str,
    me: &Person,
    all: &[DiscountRequest],
    now: i64,
    tab: Tab,
    here: &str,
) -> Frame {
    let home = format!("{base}/as/{}", me.id);
    let badge = |n: usize| if n == 0 { String::new() } else { n.to_string() };
    let link = |label, url: String, active, badge: String, hot| NavLink {
        label,
        url,
        active,
        badge,
        hot,
    };
    let mut nav = vec![];
    match me.role {
        Role::Rep => {
            let back = all
                .iter()
                .filter(|r| r.rep == me.id && r.status(now) == Status::SentBack)
                .count();
            nav.push(link(
                "My requests",
                home.clone(),
                tab == Tab::Home,
                badge(back),
                true,
            ));
            nav.push(link(
                "New request",
                format!("{home}/new"),
                tab == Tab::New,
                String::new(),
                false,
            ));
        }
        Role::Manager | Role::Vp | Role::Cfo => {
            let waiting: Vec<&DiscountRequest> =
                all.iter().filter(|r| r.is_waiting_for(me.id)).collect();
            let hot = waiting.iter().any(|r| r.is_overdue(now));
            nav.push(link(
                "Waiting for me",
                home.clone(),
                tab == Tab::Home,
                badge(waiting.len()),
                hot,
            ));
            if me.sees_all() {
                nav.push(link(
                    "All requests",
                    format!("{home}/all"),
                    tab == Tab::All,
                    String::new(),
                    false,
                ));
            }
        }
        Role::Ops => nav.push(link(
            "All requests",
            format!("{home}/all"),
            tab == Tab::All,
            String::new(),
            false,
        )),
    }
    Frame {
        base: base.into(),
        people: PEOPLE
            .iter()
            .map(|p| Choice {
                value: p.id.into(),
                label: format!("{} ({})", p.name, p.role_name()),
                selected: p.id == me.id,
            })
            .collect(),
        nav,
        here: here.into(),
        me: me.id.into(),
        home,
    }
}

/// "DR" for Daniel Reyes.
fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|w| w.chars().next())
        .collect()
}

fn first_name(name: &str) -> &str {
    name.split_whitespace().next().unwrap_or(name)
}

/// "17.5%" for 175 tenths.
fn tenths(t: u64) -> String {
    format!("{}.{}%", t / 10, t % 10)
}

// ---- Request lists -------------------------------------------------------

pub struct Mark {
    pub initials: String,
    pub state: &'static str,
    pub title: String,
}

pub struct Row {
    pub url: String,
    pub reference: String,
    pub customer: String,
    pub deal: String,
    pub discount: String,
    pub net_arr: String,
    pub rep: String,
    pub status_key: &'static str,
    pub status: &'static str,
    pub where_: String,
    pub age: String,
    pub overdue: bool,
    pub auto: bool,
    pub marks: Vec<Mark>,
}

pub fn rows<'a>(
    list: impl IntoIterator<Item = &'a DiscountRequest>,
    home: &str,
    me: &str,
    now: i64,
) -> Vec<Row> {
    list.into_iter().map(|r| row(r, home, me, now)).collect()
}

fn row(r: &DiscountRequest, home: &str, me: &str, now: i64) -> Row {
    let status = r.status(now);
    let (where_, age) = place(r, status, me, now);
    Row {
        url: format!("{home}/r/{}", r.number),
        reference: r.reference(),
        customer: r.terms.customer.clone(),
        deal: format!(
            "{}, {} seats, {}",
            r.terms.plan.label(),
            group(u64::from(r.terms.seats)),
            domain::years(r.terms.years)
        ),
        discount: r.terms.discount.to_string(),
        net_arr: dollars(r.numbers.net_arr),
        rep: people::name(&r.rep).into(),
        status_key: status.key(),
        status: status.label(),
        where_,
        age,
        overdue: r.is_overdue(now),
        auto: by_policy(r),
        marks: marks(r, status),
    }
}

fn by_policy(r: &DiscountRequest) -> bool {
    r.steps.is_empty() && r.decision.as_ref().is_some_and(|d| d.by_policy)
}

/// Where a request is, after its status ("Waiting" "for Daniel Reyes
/// (Manager)"), and how long it has been there.
fn place(r: &DiscountRequest, status: Status, me: &str, now: i64) -> (String, String) {
    let last = r.history.last();
    let last_by = last
        .map(|e| people::name(&e.by).to_string())
        .unwrap_or_default();
    let last_day = last.map(|e| time::day(&e.at)).unwrap_or_default();
    let until = r.valid_until().map(time::day).unwrap_or_default();
    match status {
        Status::Waiting => match r.waiting() {
            Some((step, since)) => (
                if step.person == me {
                    "for you".to_string()
                } else {
                    format!(
                        "for {} ({})",
                        people::name(&step.person),
                        step.approver.label()
                    )
                },
                format!("waiting {}", time::duration(now - time::secs(since))),
            ),
            None => (String::new(), String::new()),
        },
        Status::SentBack => (format!("by {last_by}"), "to change and send again".into()),
        Status::Approved if by_policy(r) => {
            (format!("until {until}"), "automatically, by policy".into())
        }
        Status::Approved => (format!("until {until}"), String::new()),
        Status::Expired => (format!("on {until}"), String::new()),
        Status::Rejected => (format!("by {last_by}"), String::new()),
        Status::Won | Status::Lost | Status::Withdrawn => (format!("on {last_day}"), String::new()),
    }
}

/// The state of each approval step: done, next, rejected, back or later.
fn step_states(r: &DiscountRequest, status: Status) -> Vec<&'static str> {
    let first_open = r.steps.iter().position(|s| s.approved_at.is_empty());
    (0..r.steps.len())
        .map(|i| {
            if !r.steps[i].approved_at.is_empty() {
                "done"
            } else if Some(i) == first_open {
                match status {
                    Status::Waiting => "next",
                    Status::Rejected => "rejected",
                    Status::SentBack => "back",
                    Status::Approved
                    | Status::Expired
                    | Status::Won
                    | Status::Lost
                    | Status::Withdrawn => "later",
                }
            } else {
                "later"
            }
        })
        .collect()
}

fn marks(r: &DiscountRequest, status: Status) -> Vec<Mark> {
    r.steps
        .iter()
        .zip(step_states(r, status))
        .map(|(s, state)| {
            let name = people::name(&s.person);
            Mark {
                initials: initials(name),
                state,
                title: format!("{name} ({}): {state}", s.approver.label()),
            }
        })
        .collect()
}

// ---- Who are you ---------------------------------------------------------

pub struct PickPerson {
    pub name: &'static str,
    pub title: String,
    pub url: String,
}

pub struct PickGroup {
    pub label: &'static str,
    pub people: Vec<PickPerson>,
}

#[derive(Template)]
#[template(path = "desk/pick.html")]
pub struct PickPage {
    pub groups: Vec<PickGroup>,
}

pub fn pick_page(base: &str) -> PickPage {
    let group = |label, roles: &[Role]| PickGroup {
        label,
        people: PEOPLE
            .iter()
            .filter(|p| roles.contains(&p.role))
            .map(|p| PickPerson {
                name: p.name,
                title: p.title(),
                url: format!("{base}/as/{}", p.id),
            })
            .collect(),
    };
    PickPage {
        groups: vec![
            group("Reps", &[Role::Rep]),
            group("Managers", &[Role::Manager]),
            group(
                "VP, finance and sales ops",
                &[Role::Vp, Role::Cfo, Role::Ops],
            ),
        ],
    }
}

// ---- Home ----------------------------------------------------------------

pub struct Section {
    pub title: String,
    pub note: String,
    pub empty: String,
    pub rows: Vec<Row>,
}

#[derive(Template)]
#[template(path = "desk/home.html")]
pub struct HomePage {
    pub frame: Frame,
    pub heading: String,
    pub intro: String,
    pub new_url: String,
    pub sections: Vec<Section>,
}

pub fn home_page(frame: Frame, me: &Person, all: &[DiscountRequest], now: i64) -> HomePage {
    let home = frame.home.clone();
    let section = |title: &str, note: &str, empty: &str, list: Vec<&DiscountRequest>| Section {
        title: title.into(),
        note: note.into(),
        empty: empty.into(),
        rows: rows(list, &home, me.id, now),
    };
    // `all` is newest first.
    match me.role {
        Role::Rep => {
            let mine: Vec<&DiscountRequest> = all.iter().filter(|r| r.rep == me.id).collect();
            let with = |statuses: &[Status]| -> Vec<&DiscountRequest> {
                mine.iter()
                    .copied()
                    .filter(|r| statuses.contains(&r.status(now)))
                    .collect()
            };
            let back = with(&[Status::SentBack]);
            let mut sections = vec![];
            if !back.is_empty() {
                sections.push(section(
                    "Sent back to you",
                    "Change them and send them again, or withdraw them.",
                    "",
                    back,
                ));
            }
            sections.push(section(
                "Open",
                "Waiting for approval, or approved and not closed yet.",
                "Nothing open. Need a discount? Start a new request.",
                with(&[Status::Waiting, Status::Approved]),
            ));
            sections.push(section(
                "Done",
                "",
                "Nothing here yet.",
                with(&[
                    Status::Won,
                    Status::Lost,
                    Status::Expired,
                    Status::Rejected,
                    Status::Withdrawn,
                ]),
            ));
            HomePage {
                heading: "My requests".into(),
                intro: format!(
                    "Hi {}. Here's where each of your requests is.",
                    first_name(me.name)
                ),
                new_url: format!("{home}/new"),
                sections,
                frame,
            }
        }
        Role::Manager | Role::Vp | Role::Cfo | Role::Ops => {
            let mut waiting: Vec<&DiscountRequest> =
                all.iter().filter(|r| r.is_waiting_for(me.id)).collect();
            waiting.sort_by_key(|r| r.waiting().map(|(_, since)| time::secs(since)).unwrap_or(0));
            let overdue = waiting.iter().filter(|r| r.is_overdue(now)).count();
            let intro = match (waiting.len(), overdue) {
                (0, _) => "Nothing is waiting for your decision.".to_string(),
                (n, 0) => format!("{n} waiting for your decision, oldest first."),
                (n, o) => format!(
                    "{n} waiting for your decision, oldest first. {o} waiting more than 24 hours, marked red."
                ),
            };
            let mut sections = vec![section("", "", "You're all caught up.", waiting)];
            if me.role == Role::Manager {
                let team: Vec<&DiscountRequest> = all
                    .iter()
                    .filter(|r| {
                        people::find(&r.rep)
                            .is_some_and(|p| Rep::of(p).is_some_and(|rep| rep.manager.id == me.id))
                    })
                    .take(50)
                    .collect();
                sections.push(section(
                    &format!("Your team, {}", me.team),
                    "All their requests, newest first.",
                    "Your team hasn't asked for anything yet.",
                    team,
                ));
            } else {
                let decided: Vec<&DiscountRequest> = all
                    .iter()
                    .filter(|r| {
                        r.history.iter().any(|e| {
                            e.by == me.id
                                && matches!(
                                    e.action,
                                    domain::Action::Approved
                                        | domain::Action::Rejected
                                        | domain::Action::SentBack
                                )
                        })
                    })
                    .take(30)
                    .collect();
                sections.push(section(
                    "Decided by you",
                    "Newest first.",
                    "You haven't decided anything yet.",
                    decided,
                ));
            }
            HomePage {
                heading: "Waiting for me".into(),
                intro,
                new_url: String::new(),
                sections,
                frame,
            }
        }
    }
}

// ---- One request ---------------------------------------------------------

pub struct ChainStep {
    pub role: &'static str,
    pub name: String,
    pub reasons: String,
    pub state: &'static str,
    pub mark: String,
    pub when: String,
    pub overdue: bool,
}

pub struct HistoryRow {
    pub at: String,
    pub when: String,
    pub who: String,
    pub action: &'static str,
    pub key: &'static str,
    pub comment: String,
    pub note: String,
    pub terms: String,
    pub changed: bool,
}

pub struct Fact {
    pub label: &'static str,
    pub value: String,
    /// Off our standard terms.
    pub off: bool,
}

pub struct NumbersView {
    pub list: String,
    pub discount: String,
    pub discount_total: String,
    pub net_arr: String,
    pub tcv: String,
}

pub struct Detail {
    pub reference: String,
    pub customer: String,
    pub link: String,
    pub rep: String,
    pub rep_first: String,
    pub rep_title: String,
    pub created: String,
    pub status_key: &'static str,
    pub status: &'static str,
    pub where_: String,
    pub chain: Vec<ChainStep>,
    pub no_route: String,
    pub numbers: NumbersView,
    pub years: String,
    pub facts: Vec<Fact>,
    pub justification: String,
    pub history: Vec<HistoryRow>,
}

/// What the person looking can do here.
#[derive(Default)]
pub struct Actions {
    /// "/discounts/as/tom/r/7": the forms post below it.
    pub url: String,
    pub decide: bool,
    pub mine: bool,
    pub mine_heading: String,
    pub mine_text: String,
    pub change: bool,
    pub renew: bool,
    pub renew_url: String,
    pub won: bool,
    pub lost: bool,
    pub withdraw: bool,
    /// Why the last try didn't work, and what was typed.
    pub error: String,
    pub comment: String,
}

#[derive(Template)]
#[template(path = "desk/request.html")]
pub struct RequestPage {
    pub frame: Frame,
    pub r: Detail,
    pub actions: Actions,
}

pub fn request_page(
    frame: Frame,
    me: &Person,
    r: &DiscountRequest,
    now: i64,
    error: String,
    comment: String,
) -> RequestPage {
    let status = r.status(now);
    let (where_, _) = place(r, status, me.id, now);
    let rep = people::find(&r.rep);
    let rep_name = people::name(&r.rep).to_string();
    let last = r.history.last();
    let last_at = last.map(|e| time::when_short(&e.at)).unwrap_or_default();

    let chain = r
        .steps
        .iter()
        .zip(step_states(r, status))
        .enumerate()
        .map(|(i, (s, state))| {
            let (mark, when) = match state {
                "done" => (
                    "✓".to_string(),
                    format!("Approved {}", time::when_short(&s.approved_at)),
                ),
                "next" => {
                    let since = r
                        .waiting()
                        .map(|(_, since)| time::secs(since))
                        .unwrap_or(now);
                    (
                        (i + 1).to_string(),
                        format!("Next: waiting {}", time::duration(now - since)),
                    )
                }
                "rejected" => ("✕".to_string(), format!("Rejected {last_at}")),
                "back" => ("↩".to_string(), format!("Sent back {last_at}")),
                _ => (
                    (i + 1).to_string(),
                    if status == Status::Waiting {
                        "Later".to_string()
                    } else {
                        "Not reached".to_string()
                    },
                ),
            };
            ChainStep {
                role: s.approver.label(),
                name: people::name(&s.person).into(),
                reasons: s.reasons.join(", "),
                state,
                mark,
                when,
                overdue: state == "next" && r.is_overdue(now),
            }
        })
        .collect();

    let no_route = match &r.decision {
        Some(d) if d.by_policy => format!(
            "No approval needed: up to 10%, annually upfront, Net 30. Approved by policy on {}.",
            time::when(&d.at)
        ),
        _ => String::new(),
    };

    let mut history: Vec<HistoryRow> = vec![];
    let mut previous: Option<&domain::Terms> = None;
    for e in &r.history {
        history.push(HistoryRow {
            at: e.at.clone(),
            when: time::when(&e.at),
            who: if e.by == POLICY {
                "the discount policy".into()
            } else {
                people::name(&e.by).into()
            },
            action: e.action.label(),
            key: e.action.key(),
            comment: e.comment.clone(),
            note: e.note.clone(),
            terms: format!(
                "{}. Net ARR {}, TCV {}.",
                e.terms.summary(),
                dollars(e.numbers.net_arr),
                dollars(e.numbers.tcv)
            ),
            changed: previous.is_some_and(|p| *p != e.terms),
        });
        previous = Some(&e.terms);
    }
    if status == Status::Expired {
        let until = r.valid_until().unwrap_or("");
        history.push(HistoryRow {
            at: until.into(),
            when: time::when(until),
            who: "the discount policy".into(),
            action: "Approval expired",
            key: "expired",
            comment: String::new(),
            note: format!(
                "{} days passed without the deal closing.",
                domain::VALID_DAYS
            ),
            terms: String::new(),
            changed: false,
        });
    }

    let t = &r.terms;
    let facts = vec![
        Fact {
            label: "Plan",
            value: format!(
                "{}, {} per user a year",
                t.plan.label(),
                dollars(t.plan.price())
            ),
            off: false,
        },
        Fact {
            label: "Seats",
            value: group(u64::from(t.seats)),
            off: false,
        },
        Fact {
            label: "Contract",
            value: domain::years(t.years),
            off: false,
        },
        Fact {
            label: "Discount",
            value: t.discount.to_string(),
            off: false,
        },
        Fact {
            label: "Billing",
            value: t.billing.label().into(),
            off: t.billing != Billing::Annual,
        },
        Fact {
            label: "Payment terms",
            value: t.payment.label().into(),
            off: t.payment != Payment::Net30,
        },
        Fact {
            label: "Competitor",
            value: if t.competitor.is_empty() {
                "None".into()
            } else {
                t.competitor.clone()
            },
            off: false,
        },
        Fact {
            label: "Expected close",
            value: time::day(&t.close_date),
            off: false,
        },
    ];

    let url = format!("{}/r/{}", frame.home, r.number);
    let mut actions = Actions {
        url: url.clone(),
        decide: status == Status::Waiting && r.is_waiting_for(me.id),
        error,
        comment,
        ..Actions::default()
    };
    if r.rep == me.id {
        let waiting_for = r
            .waiting()
            .map(|(s, _)| people::name(&s.person).to_string())
            .unwrap_or_default();
        match status {
            Status::Waiting => {
                actions.mine_heading = "Your request".into();
                actions.mine_text =
                    format!("Waiting for {waiting_for}. You can withdraw it while it waits.");
                actions.withdraw = true;
            }
            Status::SentBack => {
                let by = last
                    .map(|e| people::name(&e.by).to_string())
                    .unwrap_or_default();
                let what = last.map(|e| e.comment.clone()).unwrap_or_default();
                actions.mine_heading = format!("Sent back to you by {by}");
                actions.mine_text = format!(
                    "\u{201c}{what}\u{201d} Change it and send it again: the approvals start over from the first approver."
                );
                actions.change = true;
                actions.withdraw = true;
            }
            Status::Approved => {
                actions.mine_heading = format!(
                    "Approved until {}",
                    time::day(r.valid_until().unwrap_or(""))
                );
                actions.mine_text = "Mark the deal won or lost when it's done. If it isn't closed by then, the approval expires by itself.".into();
                actions.won = true;
                actions.lost = true;
                actions.withdraw = true;
            }
            Status::Expired => {
                actions.mine_heading = format!(
                    "The approval expired on {}",
                    time::day(r.valid_until().unwrap_or(""))
                );
                actions.mine_text = "To win the deal at this discount, ask again: the new request goes through approval like any other.".into();
                actions.renew = true;
                actions.renew_url = format!("{}/new?from={}", frame.home, r.number);
                actions.lost = true;
            }
            Status::Won | Status::Lost | Status::Rejected | Status::Withdrawn => {}
        }
        actions.mine =
            actions.change || actions.renew || actions.won || actions.lost || actions.withdraw;
    }

    RequestPage {
        r: Detail {
            reference: r.reference(),
            customer: t.customer.clone(),
            link: t.link.clone(),
            rep_first: first_name(&rep_name).into(),
            rep_title: rep.map(|p| p.title()).unwrap_or_default(),
            rep: rep_name,
            created: time::when(&r.created_at),
            status_key: status.key(),
            status: status.label(),
            where_,
            chain,
            no_route,
            numbers: NumbersView {
                list: dollars(r.numbers.list),
                discount: dollars(r.numbers.discount),
                discount_total: dollars(r.numbers.discount_total),
                net_arr: dollars(r.numbers.net_arr),
                tcv: dollars(r.numbers.tcv),
            },
            years: domain::years(t.years),
            facts,
            justification: t.justification.clone(),
            history,
        },
        actions,
        frame,
    }
}

// ---- New request, or change one -------------------------------------------

pub struct QuoteApprover {
    pub name: String,
    pub reasons: String,
}

pub struct QuoteView {
    pub ready: bool,
    pub list: String,
    pub discount: String,
    pub net_arr: String,
    pub tcv: String,
    pub years: String,
    pub problem: String,
    pub route_ready: bool,
    pub approvers: Vec<QuoteApprover>,
}

pub fn quote(preview: Preview, years: &str) -> QuoteView {
    let problem = match &preview.discount_error {
        Some(e) => e.to_string(),
        None => "Fill in plan, seats, contract length and discount to see the numbers.".into(),
    };
    let route_ready = preview.steps.is_some();
    let approvers = preview
        .steps
        .unwrap_or_default()
        .iter()
        .map(|s| QuoteApprover {
            name: format!("{} ({})", people::name(&s.person), s.approver.label()),
            reasons: s.reasons.join(", "),
        })
        .collect();
    let years = match years.trim() {
        "1" => "1 year".to_string(),
        y => format!("{y} years"),
    };
    match preview.numbers {
        Some(n) => QuoteView {
            ready: true,
            list: dollars(n.list),
            discount: dollars(n.discount),
            net_arr: dollars(n.net_arr),
            tcv: dollars(n.tcv),
            years,
            problem,
            route_ready,
            approvers,
        },
        None => QuoteView {
            ready: false,
            list: String::new(),
            discount: String::new(),
            net_arr: String::new(),
            tcv: String::new(),
            years,
            problem,
            route_ready,
            approvers,
        },
    }
}

#[derive(Template)]
#[template(path = "desk/form.html", blocks = ["quote"])]
pub struct FormPage {
    pub frame: Frame,
    pub heading: String,
    pub intro: String,
    pub action: String,
    pub quote_url: String,
    pub cancel_url: String,
    pub submit_label: &'static str,
    pub input: TermsInput,
    pub errors: FormErrors,
    pub quote: QuoteView,
    pub plans: Vec<Choice>,
    pub years: Vec<Choice>,
    pub billings: Vec<Choice>,
    pub payments: Vec<Choice>,
    /// Changing a request that was sent back.
    pub resubmit: bool,
    pub note: String,
    pub note_error: String,
    pub today: String,
}

/// Which form: a new request, or changing request `n`.
#[derive(Clone, Copy)]
pub enum FormMode {
    New,
    Change(u32),
}

pub fn form_page(
    frame: Frame,
    rep: Rep,
    mode: FormMode,
    input: TermsInput,
    errors: FormErrors,
    note: String,
    today: &str,
) -> FormPage {
    let choice = |value: &str, label: String, selected: &str| Choice {
        selected: value == selected.trim(),
        value: value.into(),
        label,
    };
    let home = frame.home.clone();
    let (heading, intro, action, cancel_url, submit_label) = match mode {
        FormMode::New => (
            "New discount request".to_string(),
            format!(
                "The numbers and who approves update as you type. Your manager is {}.",
                rep.manager.name
            ),
            format!("{home}/requests"),
            home.clone(),
            "Send for approval",
        ),
        FormMode::Change(n) => (
            format!("Change D-{n} and send it again"),
            "The approvals start over from the first approver, for the new numbers.".to_string(),
            format!("{home}/r/{n}/change"),
            format!("{home}/r/{n}"),
            "Send again",
        ),
    };
    FormPage {
        quote: quote(domain::preview(rep, &input), &input.years),
        plans: Plan::ALL
            .iter()
            .map(|p| {
                choice(
                    p.key(),
                    format!("{} {}", p.label(), dollars(p.price())),
                    &input.plan,
                )
            })
            .collect(),
        years: ["1", "2", "3"]
            .iter()
            .map(|y| choice(y, domain::years(y.parse().unwrap_or(1)), &input.years))
            .collect(),
        billings: Billing::ALL
            .iter()
            .map(|b| choice(b.key(), b.label().into(), &input.billing))
            .collect(),
        payments: Payment::ALL
            .iter()
            .map(|p| choice(p.key(), p.label().into(), &input.payment))
            .collect(),
        heading,
        intro,
        action,
        quote_url: format!("{home}/quote"),
        cancel_url,
        submit_label,
        resubmit: matches!(mode, FormMode::Change(_)),
        input,
        errors,
        note_error: if domain::parse_comment(&note).is_none() {
            format!("Keep the note to {} characters.", domain::COMMENT_MAX)
        } else {
            String::new()
        },
        note,
        today: today.into(),
        frame,
    }
}

// ---- All requests and the quarter ------------------------------------------

pub struct Tile {
    pub label: &'static str,
    pub value: String,
    pub sub: String,
}

pub struct RepLine {
    pub name: &'static str,
    pub team: &'static str,
    pub count: usize,
    pub avg_discount: String,
    pub approval: String,
    pub given: String,
}

#[derive(Template)]
#[template(path = "desk/all.html")]
pub struct AllPage {
    pub frame: Frame,
    pub quarter: String,
    pub q: String,
    pub prev_url: String,
    pub next_url: String,
    pub tiles: Vec<Tile>,
    pub reps: Vec<RepLine>,
    pub statuses: Vec<Choice>,
    pub rep_filter: Vec<Choice>,
    pub filtered: bool,
    pub count_text: String,
    pub rows: Vec<Row>,
}

/// The filters on the list: a status and a rep, or all.
pub struct Filter {
    pub status: Option<Status>,
    pub rep: Option<&'static Person>,
}

fn rate(t: &Tally) -> String {
    t.approval_rate()
        .map(|r| format!("{r}%"))
        .unwrap_or_else(|| "–".into())
}

fn avg(t: &Tally) -> String {
    t.avg_discount().map(tenths).unwrap_or_else(|| "–".into())
}

pub fn all_page(
    frame: Frame,
    all: &[DiscountRequest],
    stats: QuarterStats,
    current: (i64, i64),
    filter: Filter,
    now: i64,
) -> AllPage {
    let (y, q) = (stats.year, stats.quarter);
    let key = |(y, q): (i64, i64)| format!("{y}-q{q}");
    let quarter_url = |yq| format!("{}/all?q={}", frame.home, key(yq));
    let s = &stats.all;
    let tiles = vec![
        Tile {
            label: "Requests",
            value: s.count.to_string(),
            sub: "made this quarter".into(),
        },
        Tile {
            label: "Average discount",
            value: avg(s),
            sub: "per request".into(),
        },
        Tile {
            label: "Approval rate",
            value: rate(s),
            sub: format!("{} approved of {} decided", s.approved, s.decided),
        },
        Tile {
            label: "Time to decision",
            value: s
                .avg_decision_secs()
                .map(time::duration)
                .unwrap_or_else(|| "–".into()),
            sub: format!(
                "average, request to final decision, {} decided by people",
                s.decided_by_people
            ),
        },
        Tile {
            label: "Given away",
            value: dollars_short(s.given),
            sub: format!(
                "in discounts over the contract term, on deals approved or won ({} won)",
                dollars_short(s.given_won)
            ),
        },
    ];
    let reps = stats
        .reps
        .iter()
        .map(|(p, t)| RepLine {
            name: p.name,
            team: p.team,
            count: t.count,
            avg_discount: avg(t),
            approval: rate(t),
            given: dollars(t.given),
        })
        .collect();
    let list: Vec<&DiscountRequest> = all
        .iter()
        .filter(|r| filter.status.is_none_or(|s| r.status(now) == s))
        .filter(|r| filter.rep.is_none_or(|p| r.rep == p.id))
        .collect();
    let filtered = filter.status.is_some() || filter.rep.is_some();
    let count_text = match (list.len(), filtered) {
        (1, _) => "1 request".to_string(),
        (n, false) => format!("{n} requests, newest first"),
        (n, true) => format!("{n} requests match, newest first"),
    };
    let mut statuses = vec![Choice {
        value: String::new(),
        label: "Any status".into(),
        selected: filter.status.is_none(),
    }];
    statuses.extend(Status::ALL.iter().map(|s| Choice {
        value: s.key().into(),
        label: s.label().into(),
        selected: filter.status == Some(*s),
    }));
    let mut rep_filter = vec![Choice {
        value: String::new(),
        label: "Any rep".into(),
        selected: filter.rep.is_none(),
    }];
    rep_filter.extend(people::reps().map(|p| Choice {
        value: p.id.into(),
        label: p.name.into(),
        selected: filter.rep.is_some_and(|f| f.id == p.id),
    }));
    AllPage {
        quarter: format!("Q{q} {y}"),
        q: key((y, q)),
        prev_url: quarter_url(stats::shift(y, q, -1)),
        next_url: if (y, q) < current {
            quarter_url(stats::shift(y, q, 1))
        } else {
            String::new()
        },
        tiles,
        reps,
        statuses,
        rep_filter,
        filtered,
        count_text,
        rows: rows(list, &frame.home, &frame.me, now),
        frame,
    }
}

// ---- Messages ------------------------------------------------------------

#[derive(Template)]
#[template(path = "desk/notice.html")]
pub struct NoticePage {
    pub frame: Frame,
    pub heading: String,
    pub text: String,
    pub link_url: String,
    pub link_label: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::desk::examples::examples;

    const NOW: &str = "2026-11-13T10:00:00+01:00";

    fn now() -> i64 {
        time::secs(NOW)
    }

    fn all() -> Vec<DiscountRequest> {
        let mut all = examples(NOW);
        all.reverse();
        all
    }

    fn frame_for(id: &str, tab: Tab) -> Frame {
        frame(
            "/discounts",
            people::find(id).unwrap(),
            &all(),
            now(),
            tab,
            "",
        )
    }

    #[test]
    fn the_bar_counts_what_waits() {
        let f = frame_for("daniel", Tab::Home);
        assert_eq!(f.nav.len(), 1);
        assert_eq!(f.nav[0].badge, "2");
        assert!(f.nav[0].hot);
        let f = frame_for("marcus", Tab::Home);
        assert_eq!(f.nav[0].badge, "1");
        assert!(!f.nav[0].hot);
        assert_eq!(f.nav[1].url, "/discounts/as/marcus/all");
        let f = frame_for("priya", Tab::Home);
        assert_eq!(f.nav[0].badge, "1");
        assert!(f.people.iter().any(|c| c.selected && c.value == "priya"));
    }

    #[test]
    fn waiting_for_me_is_oldest_first_and_overdue_stands_out() {
        let me = people::find("daniel").unwrap();
        let page = home_page(frame_for("daniel", Tab::Home), me, &all(), now());
        let waiting = &page.sections[0].rows;
        assert_eq!(
            waiting
                .iter()
                .map(|r| r.reference.as_str())
                .collect::<Vec<_>>(),
            ["D-5", "D-7"]
        );
        assert!(waiting[0].overdue);
        assert_eq!(waiting[0].age, "waiting 31 h");
        assert_eq!(waiting[0].where_, "for you");
        let html = page.render().unwrap();
        assert!(html.contains("row overdue"));
        assert!(html.contains("Meridian Freight"));
        assert!(html.contains("1 waiting more than 24 hours"));
        // The team list has only EMEA requests.
        assert!(
            page.sections[1]
                .rows
                .iter()
                .all(|r| r.rep == "Priya Nair" || r.rep == "Tom Weber")
        );
    }

    #[test]
    fn a_rep_sees_their_own_requests() {
        let me = people::find("priya").unwrap();
        let page = home_page(frame_for("priya", Tab::Home), me, &all(), now());
        assert_eq!(page.sections[0].title, "Sent back to you");
        let html = page.render().unwrap();
        assert!(html.contains("Halden &#38; Co."), "{html}");
        assert!(html.contains("Oakridge Software"));
        assert!(!html.contains("Meridian Freight"));
    }

    #[test]
    fn a_request_shows_numbers_route_and_history() {
        let me = people::find("daniel").unwrap();
        let r = &examples(NOW)[4];
        let page = request_page(
            frame_for("daniel", Tab::Other),
            me,
            r,
            now(),
            String::new(),
            String::new(),
        );
        assert!(page.actions.decide);
        assert!(!page.actions.mine);
        let states: Vec<_> = page.r.chain.iter().map(|c| c.state).collect();
        assert_eq!(states, ["next", "later", "later"]);
        assert!(page.r.chain[0].overdue);
        assert_eq!(page.r.chain[1].reasons, "net ARR over $100,000");
        let html = page.render().unwrap();
        assert!(html.contains("$100,300"));
        assert!(html.contains("Your decision"));
        assert!(html.contains("Quarterly"));
    }

    #[test]
    fn others_cant_decide() {
        let r = &examples(NOW)[4];
        for id in ["marcus", "tom", "hannah"] {
            let page = request_page(
                frame_for(id, Tab::Other),
                people::find(id).unwrap(),
                r,
                now(),
                String::new(),
                String::new(),
            );
            assert!(!page.actions.decide, "{id}");
        }
        let tom = request_page(
            frame_for("tom", Tab::Other),
            people::find("tom").unwrap(),
            r,
            now(),
            String::new(),
            String::new(),
        );
        assert!(tom.actions.withdraw && !tom.actions.won);
    }

    #[test]
    fn sent_back_shows_what_to_change() {
        let r = &examples(NOW)[3];
        let me = people::find("priya").unwrap();
        let page = request_page(
            frame_for("priya", Tab::Other),
            me,
            r,
            now(),
            String::new(),
            String::new(),
        );
        assert!(page.actions.change);
        assert!(page.actions.mine_text.contains("Go to 15%"));
        assert_eq!(page.r.chain[0].state, "back");
    }

    #[test]
    fn expired_approvals_show_in_the_history() {
        let r = &examples(NOW)[0];
        let later = now() + 31 * time::DAY;
        let me = people::find("tom").unwrap();
        let page = request_page(
            frame_for("tom", Tab::Other),
            me,
            r,
            later,
            String::new(),
            String::new(),
        );
        assert_eq!(page.r.status, "Expired");
        assert_eq!(page.r.history.last().unwrap().action, "Approval expired");
        assert!(page.actions.renew && page.actions.lost && !page.actions.won);
    }

    #[test]
    fn the_quote_block_renders_alone() {
        let rep = Rep::of(people::find("priya").unwrap()).unwrap();
        let mut input = TermsInput::blank();
        input.plan = "business".into();
        input.seats = "120".into();
        input.years = "2".into();
        input.discount = "18".into();
        let page = form_page(
            frame_for("priya", Tab::New),
            rep,
            FormMode::New,
            input,
            FormErrors::default(),
            String::new(),
            "2026-11-13",
        );
        let quote = page.as_quote().render().unwrap();
        assert!(quote.contains("$58,056"));
        assert!(quote.contains("$116,112"));
        assert!(quote.contains("Daniel Reyes (Manager)"));
        let html = page.render().unwrap();
        assert!(html.contains(r#"value="business" checked"#));
        assert!(html.contains(r#"action="/discounts/as/priya/requests""#));
    }

    #[test]
    fn the_quarter_page() {
        let all = all();
        let stats = stats::quarter_stats(&all, 2026, 4, now());
        let filter = Filter {
            status: Some(Status::Waiting),
            rep: None,
        };
        let page = all_page(
            frame_for("hannah", Tab::All),
            &all,
            stats,
            (2026, 4),
            filter,
            now(),
        );
        assert_eq!(page.rows.len(), 3);
        assert_eq!(page.tiles[0].value, "7");
        assert!(page.next_url.is_empty());
        assert_eq!(page.prev_url, "/discounts/as/hannah/all?q=2026-q3");
        let html = page.render().unwrap();
        assert!(html.contains("Q4 2026"));
        assert!(html.contains(r#"<option value="waiting" selected>"#));
    }
}
