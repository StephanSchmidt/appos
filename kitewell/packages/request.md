Hi! I'm Hannah, I run sales operations at Kitewell. Today a customer who wants to buy emails a rep, the rep copies the numbers from the discount desk into a quote, and finance types it all again for the invoice. Engineering has now built booking into Kitewell itself: customers book a package on our site, and the booking lands with finance. Engineering keeps the prices and the booking, but two pages of the flow are ours in sales: the package page and the terms page. Our platform team says we can build them ourselves as an app with appos, using the booking functions engineering gives us (`kitewell:booking`). Here's what I need.

**The flow**, so you know where our pages sit: packages (ours) → configure (product's) → terms (ours) → review and book (engineering's) → set up (product's). Each page sends the customer on to the next one; the booking functions tell you where that is.

**The package page** is where every booking starts.

- First question: how many people are on your team? Then all packages for that size, side by side.
- Our packages:
  - **Starter**: up to 10 users.
  - **Team**: 3 to 100 users. Always marked "Most popular".
  - **Business**: up to 250 users, billed for at least 10.
  - **Agency package**: our own. It's Business for agencies of 25 users or more. Copy: "For agencies that plan people across clients. Business with everything in it, for teams of 25 and up." On the terms page, preselect 2 years for it.
- Show the price per user per month from the price list, and "Two months free when billed yearly."
- A package that doesn't fit the team size stays on the page, greyed out, with the reason in words ("Starter is for up to 10 users"). Take the limits from the catalog, never from your own copy: when finance changes them, our page must follow.
- More than 250 users: no packages, only "Talk to sales" with a link to sales@kitewell.example.
- Under the packages: "Got an offer code from your account executive?" A code that works takes the customer on with that offer; one that doesn't says why ("This offer expired on 31 December 2026").

**The terms page.**

- Billing: yearly (preselected), or monthly.
- Contract length: 1, 2 or 3 years. Monthly billing is for 1 year only; say so instead of hiding it.
- Payment: by invoice, Net 30.
- Show what the customer pays as the booking functions calculate it: per year, for the whole contract, and per invoice. Never calculate a price on our side.
- With an offer, the terms are the offer's. Show them, don't let anyone change them, and say "These terms come with your offer KW-…".

**Rules.** Prices, discounts and what's allowed are engineering's and finance's, and the booking functions refuse anything else. Our job is how we present and sell it. If a booking function says no, show its reason as it is.

**Look:** our site's look, clear and calm. It has to work on a phone.
