// Drafts, a catalog and bookings for tests, shaped like the booking
// functions' answers.

import type { Booking, CatalogInfo, Draft } from "../bindings/flow.ts";

export const ID = "0123456789abcdef0123456789abcdef";

export function draft(change: Partial<Draft> = {}): Draft {
  return {
    id: ID,
    package: "Team",
    plan: "team",
    users: 12,
    addOns: [],
    template: undefined,
    billing: "yearly",
    years: 1,
    paymentTerms: "net30",
    offer: undefined,
    quote: {
      lines: [{ label: "Team, 12 users, yearly", cents: 348000 }],
      billedUsers: 12,
      perYearCents: 348000,
      contractCents: 348000,
      perInvoiceCents: 348000,
      problems: [],
    },
    ...change,
  };
}

export const CATALOG: CatalogInfo = {
  plans: [],
  addOns: [{ id: "ai-assistant", name: "AI planning assistant", monthlyCents: 400, includedIn: ["business"] }],
  templates: [
    { id: "plan-a-quarter", name: "Plan a quarter", description: "Goals, projects and people for the next three months." },
    { id: "agency-capacity", name: "Agency capacity plan", description: "Clients, projects and who works on what." },
  ],
  enterpriseFrom: 251,
};

export function booking(change: Partial<Booking> = {}): Booking {
  return {
    id: ID,
    company: "Nordwind Studio",
    package: "Team",
    users: 12,
    workspace: "https://app.kitewell.example/w/nordwind-studio-a1b2c3",
    members: [{ name: "Lena Vogel", email: "lena@nordwind.example", invite: "https://app.kitewell.example/join/1" }],
    project: "Plan a quarter",
    quote: draft().quote,
    ...change,
  };
}
