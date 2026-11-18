//! The packages sales sells, and what a package page says about each:
//! plain Rust, no IO. A package is one of the platform's plans with sales's
//! name, pitch and rules on top. The plans' limits and prices come from
//! the platform (flow.catalog); the platform checks them again when a
//! package is chosen.

use crate::appos::kitewell::booking::flow::{CatalogInfo, Plan, PlanInfo};
use crate::quote::money;

/// Where customers with more users than any package write to.
pub const SALES: &str = "sales@kitewell.example";

pub struct Package {
    pub name: &'static str,
    pub plan: Plan,
    /// A label on the card, like "Most popular"; empty for none.
    pub badge: &'static str,
    pub pitch: &'static str,
    /// Sales's own minimum, above the plan's; 0 for none.
    pub from_users: u32,
    /// The contract length the terms page starts with.
    pub years: u8,
}

pub const PACKAGES: [Package; 4] = [
    Package {
        name: "Starter",
        plan: Plan::Starter,
        badge: "",
        pitch: "For a small team that plans together.",
        from_users: 0,
        years: 1,
    },
    Package {
        name: "Team",
        plan: Plan::Team,
        badge: "Most popular",
        pitch: "For teams that need to see who has time for what.",
        from_users: 0,
        years: 1,
    },
    Package {
        name: "Business",
        plan: Plan::Business,
        badge: "",
        pitch: "For companies with SSO, an audit log and a success manager.",
        from_users: 0,
        years: 1,
    },
    Package {
        name: "Agency package",
        plan: Plan::Business,
        badge: "",
        pitch: "For agencies that plan people across clients. Business with everything in it, for teams of 25 and up.",
        from_users: 25,
        years: 2,
    },
];

pub fn package(name: &str) -> Option<&'static Package> {
    PACKAGES.iter().find(|p| p.name == name)
}

/// The team size as typed: a whole number, at least 1.
pub fn users(typed: &str) -> Result<u32, &'static str> {
    match typed.trim().parse::<u32>() {
        Ok(0) => Err("A team has at least 1 person"),
        Ok(n) => Ok(n),
        Err(_) => Err("Enter your team size as a number, like 12"),
    }
}

/// Why `users` people can't book the package, or none if they can.
pub fn unavailable(
    package: &Package,
    plan: &PlanInfo,
    users: u32,
    enterprise_from: u32,
) -> Option<String> {
    let from = plan.min_users.max(package.from_users);
    if users >= enterprise_from {
        Some(format!(
            "More than {} users: talk to sales",
            enterprise_from - 1
        ))
    } else if users > plan.max_users {
        Some(format!(
            "{} is for up to {} users",
            package.name, plan.max_users
        ))
    } else if users < from {
        Some(format!("{} starts at {from} users", package.name))
    } else {
        None
    }
}

/// A package's card on the page.
pub struct Card {
    pub name: &'static str,
    pub badge: &'static str,
    pub pitch: &'static str,
    pub monthly: String,
    pub yearly: String,
    /// Like "Billed for at least 10 users"; empty when it bills each user.
    pub minimum: String,
    pub features: Vec<String>,
    /// Why it can't be chosen for this team size; empty when it can.
    pub why_not: String,
    /// The draft has it.
    pub chosen: bool,
    /// The card that stands out: the draft's package, or else the one with
    /// a badge.
    pub featured: bool,
}

/// The cards for a team size (none yet: 0), with the draft's package.
pub fn cards(catalog: &CatalogInfo, users: u32, chosen: &str) -> Vec<Card> {
    let any_chosen = PACKAGES.iter().any(|p| p.name == chosen);
    PACKAGES
        .iter()
        .filter_map(|p| {
            let plan = catalog.plans.iter().find(|i| i.plan == p.plan)?;
            let why_not = match users {
                0 => "Enter your team size above".to_string(),
                n => unavailable(p, plan, n, catalog.enterprise_from).unwrap_or_default(),
            };
            Some(Card {
                name: p.name,
                badge: p.badge,
                pitch: p.pitch,
                monthly: money(i64::from(plan.monthly_cents)),
                yearly: money(i64::from(plan.monthly_cents) * 10),
                minimum: if plan.billed_minimum > 1 {
                    format!("Billed for at least {} users", plan.billed_minimum)
                } else {
                    String::new()
                },
                features: plan.features.clone(),
                why_not,
                chosen: p.name == chosen,
                featured: if any_chosen {
                    p.name == chosen
                } else {
                    !p.badge.is_empty()
                },
            })
        })
        .collect()
}

/// The slider under the team size, as on Kitewell's pricing page: stops
/// one user apart up to 20, five apart up to 100 and ten apart above, up to
/// the largest team a package takes. A team of 7 is as easy to pick as one
/// of 170.
pub struct Slider {
    largest: u32,
}

/// A label under the slider, at a team size.
pub struct Mark {
    pub label: String,
    /// From 0 (the left end) to 1 (the right end).
    pub at: String,
}

impl Slider {
    pub fn new(catalog: &CatalogInfo) -> Slider {
        Slider {
            largest: catalog.enterprise_from.saturating_sub(1).max(100),
        }
    }

    /// The last stop, at the largest team.
    pub fn last(&self) -> u32 {
        35 + (self.largest - 100).div_ceil(10)
    }

    /// The team size at a stop.
    pub fn users(&self, stop: u32) -> u32 {
        let users = match stop {
            0..=19 => stop + 1,
            20..=35 => 25 + (stop - 20) * 5,
            _ => 100 + (stop - 35) * 10,
        };
        users.min(self.largest)
    }

    /// The stop nearest to a team size; above the largest, the last one.
    /// Of two as near, the lower.
    pub fn stop(&self, users: u32) -> u32 {
        (0..=self.last())
            .min_by_key(|&s| self.users(s).abs_diff(users))
            .unwrap_or(0)
    }

    /// A mark at 1 and at each plan's largest team, from the catalog.
    pub fn marks(&self, catalog: &CatalogInfo) -> Vec<Mark> {
        let mut sizes: Vec<u32> = catalog.plans.iter().map(|p| p.max_users).collect();
        sizes.push(1);
        sizes.sort_unstable();
        sizes.dedup();
        sizes
            .into_iter()
            .map(|n| Mark {
                label: n.to_string(),
                at: format!("{:.3}", f64::from(self.stop(n)) / f64::from(self.last())),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan(plan: Plan, min_users: u32, max_users: u32, billed_minimum: u32) -> PlanInfo {
        PlanInfo {
            plan,
            name: format!("{plan:?}"),
            monthly_cents: 2900,
            min_users,
            max_users,
            billed_minimum,
            features: vec!["Guest access".into()],
        }
    }

    fn catalog() -> CatalogInfo {
        CatalogInfo {
            plans: vec![
                plan(Plan::Starter, 1, 10, 1),
                plan(Plan::Team, 3, 100, 3),
                plan(Plan::Business, 1, 250, 10),
            ],
            add_ons: vec![],
            templates: vec![],
            enterprise_from: 251,
        }
    }

    #[test]
    fn a_team_size_greys_out_what_doesnt_fit() {
        let why: Vec<String> = cards(&catalog(), 12, "Team")
            .into_iter()
            .map(|c| c.why_not)
            .collect();
        assert_eq!(
            why,
            [
                "Starter is for up to 10 users",
                "",
                "",
                "Agency package starts at 25 users"
            ]
        );
        let why: Vec<String> = cards(&catalog(), 2, "")
            .into_iter()
            .map(|c| c.why_not)
            .collect();
        assert_eq!(why[1], "Team starts at 3 users");
        let why: Vec<String> = cards(&catalog(), 300, "")
            .into_iter()
            .map(|c| c.why_not)
            .collect();
        assert!(
            why.iter()
                .all(|w| w == "More than 250 users: talk to sales")
        );
    }

    #[test]
    fn a_card_shows_both_prices_and_the_minimum() {
        let cards = cards(&catalog(), 0, "Team");
        assert_eq!(
            (cards[1].monthly.as_str(), cards[1].yearly.as_str()),
            ("$29", "$290")
        );
        assert!(cards[1].chosen && !cards[0].chosen);
        assert_eq!(cards[2].minimum, "Billed for at least 10 users");
        assert_eq!(cards[0].why_not, "Enter your team size above");
    }

    #[test]
    fn team_size_is_a_number() {
        assert_eq!(users(" 12 "), Ok(12));
        assert!(users("0").is_err());
        assert!(users("twelve").is_err());
    }

    #[test]
    fn the_slider_is_fine_for_small_teams_and_coarse_for_large_ones() {
        let s = Slider::new(&catalog());
        assert_eq!(s.last(), 50);
        assert_eq!(
            (s.users(0), s.users(19), s.users(20), s.users(35)),
            (1, 20, 25, 100)
        );
        assert_eq!((s.users(36), s.users(50)), (110, 250));
        assert_eq!(s.stop(25), 20);
        assert_eq!(s.stop(105), 35, "of 100 and 110, the lower");
        assert_eq!(s.stop(400), 50);
        let marks: Vec<(String, String)> = s
            .marks(&catalog())
            .into_iter()
            .map(|m| (m.label, m.at))
            .collect();
        assert_eq!(marks[0], ("1".to_string(), "0.000".to_string()));
    }
}
