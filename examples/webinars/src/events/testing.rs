//! Test data shared by the module's tests.

use super::domain::{Applicant, ApplicantForm, SessionInput};
use crate::appos::kv::Unreadable;
use crate::appos::time::{HOUR, Time};

/// 2026-11-13T10:00:00+01:00.
pub const NOW: Time = Time::utc(1_794_560_400).at_offset(HOUR);

pub fn applicant(email: &str) -> Applicant {
    Applicant::parse(&ApplicantForm {
        first_name: "Ann".into(),
        last_name: "Lee".into(),
        email: email.into(),
        company: "Acme".into(),
        ..ApplicantForm::default()
    })
    .unwrap()
}

/// What a loader returns when the text under `key` isn't the JSON it reads.
pub fn unreadable(key: &str) -> Unreadable {
    Unreadable {
        key: key.into(),
        reason: "expected value at line 1 column 1".into(),
    }
}

/// 4 December 2026, 16:00 Berlin, 50 seats.
pub fn session_input() -> SessionInput {
    SessionInput {
        title: "Plan a quarter in 30 minutes".into(),
        description: "Mira walks through planning a full quarter.".into(),
        date: "2026-12-04".into(),
        time: "16:00".into(),
        start: super::clock::berlin_to_utc("2026-12-04", "16:00").unwrap(),
        minutes: 45,
        speaker: "Mira Jensen".into(),
        role: "Head of Customer Education".into(),
        seats: 50,
        join_link: "https://meet.kitewell.example/plan-a-quarter".into(),
    }
}
