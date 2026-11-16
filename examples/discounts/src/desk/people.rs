//! Who works with the desk. There are no logins yet: people pick who they
//! are, and the choice travels in the URL (/as/<id>/...).

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Rep,
    Manager,
    Vp,
    Cfo,
    Ops,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Person {
    pub id: &'static str,
    pub name: &'static str,
    pub role: Role,
    pub team: &'static str,
    /// The manager's id, for reps.
    manager: Option<&'static str>,
}

impl Person {
    /// "Rep, EMEA", "VP Sales".
    pub fn title(&self) -> String {
        match self.role {
            Role::Rep => format!("Rep, {}", self.team),
            Role::Manager => format!("Manager, {}", self.team),
            Role::Vp => "VP Sales".into(),
            Role::Cfo => "CFO".into(),
            Role::Ops => "Sales ops".into(),
        }
    }

    /// "Rep", "Manager", "VP Sales": short, for the "You are" picker.
    pub fn role_name(&self) -> &'static str {
        match self.role {
            Role::Rep => "Rep",
            Role::Manager => "Manager",
            Role::Vp => "VP Sales",
            Role::Cfo => "CFO",
            Role::Ops => "Sales ops",
        }
    }

    /// Hannah and Marcus see all requests and the quarter's numbers.
    pub fn sees_all(&self) -> bool {
        matches!(self.role, Role::Vp | Role::Ops)
    }
}

pub const PEOPLE: &[Person] = &[
    Person {
        id: "priya",
        name: "Priya Nair",
        role: Role::Rep,
        team: "EMEA",
        manager: Some("daniel"),
    },
    Person {
        id: "tom",
        name: "Tom Weber",
        role: Role::Rep,
        team: "EMEA",
        manager: Some("daniel"),
    },
    Person {
        id: "daniel",
        name: "Daniel Reyes",
        role: Role::Manager,
        team: "EMEA",
        manager: None,
    },
    Person {
        id: "sofia",
        name: "Sofia Marin",
        role: Role::Rep,
        team: "North America",
        manager: Some("grace"),
    },
    Person {
        id: "ken",
        name: "Ken Ito",
        role: Role::Rep,
        team: "North America",
        manager: Some("grace"),
    },
    Person {
        id: "grace",
        name: "Grace Kim",
        role: Role::Manager,
        team: "North America",
        manager: None,
    },
    Person {
        id: "marcus",
        name: "Marcus Hale",
        role: Role::Vp,
        team: "",
        manager: None,
    },
    Person {
        id: "elena",
        name: "Elena Petrova",
        role: Role::Cfo,
        team: "",
        manager: None,
    },
    Person {
        id: "hannah",
        name: "Hannah Boateng",
        role: Role::Ops,
        team: "",
        manager: None,
    },
];

pub fn find(id: &str) -> Option<&'static Person> {
    PEOPLE.iter().find(|p| p.id == id)
}

/// The name for an id from stored data; an id nobody has shows as itself.
pub fn name(id: &str) -> &str {
    find(id).map(|p| p.name).unwrap_or(id)
}

pub fn vp() -> &'static Person {
    find("marcus").expect("the VP is in PEOPLE")
}

pub fn cfo() -> &'static Person {
    find("elena").expect("the CFO is in PEOPLE")
}

pub fn reps() -> impl Iterator<Item = &'static Person> {
    PEOPLE.iter().filter(|p| p.role == Role::Rep)
}

/// A person who can ask for a discount: a rep, with a manager. The only
/// way to get one is `Rep::of`, so code that takes a `Rep` never has to
/// ask whether the person has a manager.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rep {
    pub person: &'static Person,
    pub manager: &'static Person,
}

impl Rep {
    pub fn of(person: &'static Person) -> Option<Rep> {
        let manager = find(person.manager?)?;
        (person.role == Role::Rep).then_some(Rep { person, manager })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reps_have_their_managers() {
        let priya = Rep::of(find("priya").unwrap()).unwrap();
        assert_eq!(priya.manager.name, "Daniel Reyes");
        let ken = Rep::of(find("ken").unwrap()).unwrap();
        assert_eq!(ken.manager.name, "Grace Kim");
    }

    #[test]
    fn only_reps_are_reps() {
        for id in ["daniel", "grace", "marcus", "elena", "hannah"] {
            assert!(Rep::of(find(id).unwrap()).is_none(), "{id}");
        }
        assert_eq!(reps().count(), 4);
    }

    #[test]
    fn who_sees_what() {
        assert!(find("hannah").unwrap().sees_all());
        assert!(find("marcus").unwrap().sees_all());
        assert!(!find("daniel").unwrap().sees_all());
    }
}
