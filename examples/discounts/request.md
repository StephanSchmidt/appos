Hi! I'm Daniel, I run sales for EMEA at Kitewell. Discount approvals happen in Slack DMs today, and it's chaos: reps ping three people, nobody knows what was actually approved, finance finds out at invoice time that someone gave 35% with monthly billing, and every quarter our VP asks me for numbers I have to dig out of Slack. Our platform team says we can now build our own small web apps with a tool called "appos", so I want a proper little discount desk. Here's how it has to work.

**People.** We don't have logins yet, so just let people pick who they are at the top, and keep that choice while they click around.
- Reps on my team (EMEA): Priya Nair, Tom Weber. Their manager: me, Daniel Reyes.
- Reps on Grace's team (North America): Sofia Marin, Ken Ito. Their manager: Grace Kim.
- VP Sales: Marcus Hale.
- CFO: Elena Petrova. She approves for finance.
- Sales ops: Hannah Boateng. She sees everything but doesn't approve.

**A request** (the rep fills it in):
- Customer name, and the link to the Salesforce opportunity (optional, but most have one)
- Plan: Team or Business. Starter is never discounted.
- Seats
- Contract length: 1, 2 or 3 years
- Discount in percent (whole numbers or halves, like 12.5)
- Billing: annually upfront (our standard), quarterly, or monthly
- Payment terms: Net 30 (standard), Net 45 or Net 60
- Competitor in the deal, if any
- Expected close date
- Justification, required. "Customer asked" is not a justification; make them write at least a real sentence.

**The numbers**, calculated for them (reps get these wrong in Slack all the time):
- List price per year: seats × our yearly list price (Team $290, Business $590 per user per year)
- Discount per year in dollars
- Net ARR: what the customer pays per year after the discount
- Total contract value: net ARR × years
- Example: 120 seats of Business, 2 years, 18% off: list $70,800, discount $12,744 a year, net ARR $58,056, TCV $116,112.

**Who approves. This is our policy and the heart of the whole thing:**
- Up to 10%: approved automatically, nobody has to click. But it's on record like everything else.
- More than 10% up to 20%: the rep's manager.
- More than 20% up to 30%: the rep's manager, then the VP.
- More than 30% up to 50%: the manager, the VP, then the CFO.
- More than 50%: not possible. The form says no.
- On top of that: billing other than annually upfront, or payment terms other than Net 30, also needs the CFO, even at 5%. A deal over $100,000 net ARR also needs the VP, whatever the discount.
- Always in this order: manager, then VP, then CFO. The next person only sees it once the one before has approved. Nobody approves the same request twice, even when two rules need them.

**What approvers can do:** approve (comment optional), reject (comment required: tell the rep why), or send it back for changes (comment required, like "go to 15% with a 2-year term and I'll sign it"). When the rep changes it and sends it again, the approvals start over from the first person, because people approved different numbers. A rep can also withdraw a request.

**After approval:** an approved discount is valid for 30 days. If the deal isn't closed by then, the approval expires by itself and the rep needs a new one. The rep marks the deal Won or Lost when it's done.

**History.** Every request keeps its full history: who did what and when, with their comment and the numbers at that moment. Nothing in the history can be edited or deleted. This is what finance audits.

**Screens:**
- "Waiting for me": what's waiting for my decision, oldest first. Anything waiting more than 24 hours stands out.
- Reps see their own requests and where each one is.
- A request: the numbers, who approved already, who's next, the history.
- For Hannah and Marcus: all requests, filtered by status or rep, and a few numbers for the quarter: how many requests, average discount (overall and per rep), approval rate, average time from request to final decision, and the dollars we gave away in discounts.

**Example data:** put in a handful of requests so it doesn't look empty on day one: one approved automatically, one waiting for me, one waiting for Marcus, one approved, one rejected, one sent back for changes. Use made-up customers.

**Look:** an internal tool we use all day: clear, dense, quick. No marketing fluff. Kitewell indigo #3B3FD8 for buttons is fine, ink #171A33 for text. It has to work on my phone; I approve deals from airports. Our design team made the icon, it's in design/icon.svg in your folder: please use it as the app's icon.

**Where it goes:** our platform team gave me this: deploy it to http://127.0.0.1:8111 (that's where you connect), and call the app `discounts`. Put me down as the owner. When it's deployed, send me the address so I can open it.

Afterwards, write me a short note: what was unclear or annoying while building it?
