Hi! I'm Nora, I run growth marketing at Kitewell and I own our pricing page. Our platform team says we can now build our own small web apps with a tool called "appos", so I'd love a proper price calculator instead of the static table we have today. I've thought about this for a while, so here is everything it needs.

**Our prices** (US dollars, per user per month, when paying monthly):
- **Starter: $12.** For small teams, 1 to 10 users.
- **Team: $29.** From 3 to 100 users. Our most popular plan, so please always mark it "Most popular".
- **Business: $59.** Up to 250 users, with a minimum of 10: a team of 6 can buy Business, but it pays for 10 users.
- **More than 250 users:** Enterprise. No price, they talk to sales.

**Paying yearly gives two months free**, so a year costs 10 months. People think in monthly numbers, so show both: the price per user per month when billed yearly ("$24.17 per user/month, billed yearly") and the total per year ("$7,250 per year"), plus what they save compared to paying monthly ("You save $1,450 a year"). Yearly should be selected when the page opens; it's what we want to sell.

**Add-on: the AI planning assistant**, $8 per user per month on Starter and Team. Business includes it at no extra cost; say "Included" there, it's a selling point for Business. The two free months apply to the add-on too.

**What visitors do:** type how many people are on their team, choose monthly or yearly, switch the AI assistant on or off. The numbers must change right away while they type or click, no "Calculate" button. Show all three plans side by side for their team size so they can compare. If a plan doesn't fit their team size, don't hide it: grey it out and say why ("Starter is for up to 10 users", "Team starts at 3 users"). When Business would bill more users than they have, say it plainly ("Billed for 10 users, the Business minimum"). Above 250 users, show only Enterprise with "Talk to sales".

**What each plan includes** (my copy, use it as is):
- Starter: Unlimited projects · Calendar and timeline views · Slack and Google Calendar integration · Email support
- Team: Everything in Starter · Workload view and capacity planning · Guest access · Priority support
- Business: Everything in Team · SSO and SCIM · Audit log · AI planning assistant included · Dedicated success manager

**People type strange things.** I've watched the recordings: 0, -5, 2.5, "twenty", 100000, or they delete the number and the field is empty. Never show a broken page, "NaN" or a price of $0. Tell them nicely what works, e.g. "Enter a number of users from 1 to 250. More than 250? Talk to sales."

**Numbers:** totals in whole dollars with thousands separators ($7,250), per-user prices with cents when needed ($24.17) and without when they're round ($29).

**Links:**
- "Start free trial" goes to `https://app.kitewell.example/signup` with their choice, like `?plan=team&users=25&billing=yearly&ai=yes`.
- "Talk to sales" goes to `https://kitewell.example/contact-sales?users=300`.
- Our paid campaigns open the calculator with UTM parameters (utm_source, utm_medium, utm_campaign, utm_content, utm_term). They must be passed on to both links, or we lose attribution.

**Sharing:** our sales team wants to send prospects a link with everything filled in ("here's your price for 40 users on Business, yearly"). So the page's address should change with the choices, and opening that address shows the same result.

**Legal:** it must not store anything about visitors. No cookies, no tracking, nothing saved. It's only a calculator.

Small print under the prices: "Prices in US dollars, excluding VAT and sales tax."

**Look:** it goes on our pricing page and into ads, and 60% of our paid traffic is on phones, so it must work great on a phone first. It should look polished and trustworthy, not like a spreadsheet. Brand: Kitewell indigo #3B3FD8 for buttons and highlights, ink #171A33 for text, white or very light backgrounds, a clean sans-serif. Our design team made the icon, it's in design/icon.svg in your folder: please use it as the app's icon.

**Where it goes:** our platform team gave me this: deploy it to http://127.0.0.1:8111 (that's where you connect), and call the app `pricing`. Put me down as the owner. When it's deployed, send me the address so I can open it.

Afterwards, write me a short note: what was unclear or annoying while building it?
