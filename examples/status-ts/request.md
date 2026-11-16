Hi! I'm Amara, I lead customer support at Kitewell. Every time something breaks, my team gets a couple of hundred tickets asking "is it down?", and we answer them one by one. I want a public status page that we in support keep up to date ourselves, so customers can check it and we can link to it from our ticket macros. During an incident the engineers are busy fixing things; they tell us in Slack what's going on and we translate it for customers. So support writes the page, not engineering. Our platform team says we can now build our own small web apps with a tool called "appos". Here's what I need.

**What customers see**

At the top, one clear sentence for the worst current state: "All systems operational", "Some systems are having problems", or "Major outage".

Then our parts, grouped the way customers think of them:
- Kitewell web app
- Mobile apps: iOS, Android
- API
- Integrations: Slack, Google Calendar, Microsoft Teams
- Notifications: Email, Push notifications

Each one shows its status: Operational, Degraded performance, Partial outage, Major outage, or Under maintenance. Always in words, never only a colour: some of our customers are colour-blind.

For each part, the last 90 days as a row of small bars, one per day, coloured by the worst thing that happened that day, and the uptime for those 90 days in percent (only partial and major outage time counts as down). Pointing at or tapping a day should say what happened that day.

**Incidents.** An incident has a title in customer words ("Some users can't log in"), the parts it affects and how badly, and a timeline of updates. Each update has a stage (Investigating, Identified, Monitoring, Resolved), a message and the time. Newest update on top. All times in UTC, written like "Nov 13, 14:05 UTC", because our customers are all over the world; "25 minutes ago" next to it is nice. Open incidents at the top of the page. Resolved ones under "Past incidents", grouped by month, with how long they lasted.

**Planned maintenance.** We announce it at least three days ahead: a title, which parts, start and end time (UTC), and what customers will notice. Before it starts it shows as "Upcoming maintenance". While the window is open, the parts show "Under maintenance" automatically: nobody on my team should have to be awake at 3 am to click a button. After the end time it's over and moves to the past. If it runs late, we post an update or open an incident.

**Our side, the desk** (only my team uses it). We don't have logins yet, so just let us pick our name from a list: Amara Okafor, Jonas Weber, Mei Tanaka, Lucas Silva. Every update records who posted it; that's for us only, customers never see names. On the desk we can:
- Open an incident: title, the affected parts and their status, the first update.
- Post an update: stage, message, and change the parts' status if needed. Optionally "Next update in 30 minutes" or "in 60 minutes", which customers then see.
- Resolve: the parts go back to Operational (we can keep one degraded if something isn't fully fixed).
- Use templates, so a stressed agent at 2 am doesn't start from a blank box. These four; they replace the part in brackets:
  - Investigating: "We're looking into reports that [what customers notice]. We'll post an update within 30 minutes."
  - Identified: "We've found the cause of [the problem] and are working on a fix."
  - Monitoring: "A fix is in place and we're watching the results. Things should be back to normal for most customers."
  - Resolved: "This is resolved: [what] works normally again. We're sorry for the trouble."
- Fix a typo: we can edit an update's text, and customers see a small "edited" next to it. We never delete an update or an incident. Customers screenshot us and quote us, so the history must stay honest.
- Schedule maintenance.

Next to the message box, please show our three writing rules as a reminder: say what customers notice, not system names ("database", "db-03", "Kubernetes"); never blame a vendor or another team; say when the next update comes.

**Start with some history** so the page doesn't look brand new:
- A resolved incident: "Delayed email notifications", 6 November 2026, 09:12 to 10:47 UTC, Email had a partial outage. Updates: Investigating at 09:12, Identified at 09:40, Monitoring at 10:15, Resolved at 10:47 (write sensible messages).
- An upcoming maintenance: "Kitewell will be read-only for up to an hour", Sunday 22 November 2026, 02:00 to 03:00 UTC, affecting the web app, both mobile apps and the API. Customers can view everything but can't make changes during the window.
- Everything else operational.

We know we can't email subscribers from this tool. A feed (RSS or Atom) that customers can add to their tools would be nice if the tool can do it; otherwise skip it.

**Look:** customers open this page when they're already annoyed, so calm, clear and quick to read. Kitewell indigo #3B3FD8 for our header and buttons, ink #171A33 for text, but the status colours should be the usual ones (green, yellow, orange, red, blue for maintenance), always with words. It has to work on phones. Our design team made the icon, it's in design/icon.svg in your folder: please use it as the app's icon.

**Where it goes:** our platform team gave me this: deploy it to http://127.0.0.1:8111 (that's where you connect), and call the app `status`. Put me down as the owner. When it's deployed, send me the address so I can open it.

Afterwards, write me a short note: what was unclear or annoying while building it?
