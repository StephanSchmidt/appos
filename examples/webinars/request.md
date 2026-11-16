Hi! I'm Lukas, I run events and webinars at Kitewell. Today we use a Google Form plus a spreadsheet, and it's a mess: people register twice, we overbooked our last workshop (54 people for 50 Zoom seats), nobody knows who's on the waitlist, and I can't tell which campaign brought people in. Our platform team says we can now build our own small web apps with a tool called "appos", so I'd like our own webinar registration. Here is what I need.

**Sessions — I manage them myself**, without asking anyone: add, edit, cancel. For each session: title, a short description (two or three sentences), date and start time (I plan everything in Berlin time), length in minutes, speaker name and role, number of seats (usually 50; our Zoom licence allows at most 100), and the Zoom join link.

Please put these three in to start with:
1. "Plan a quarter in 30 minutes": Thursday 4 December 2026, 16:00 Berlin, 45 minutes. Mira Jensen, Head of Customer Education. 50 seats. Join link https://meet.kitewell.example/plan-a-quarter. Description: "Mira walks through planning a full quarter in Kitewell, from goals to a realistic timeline. Bring your own plan and questions."
2. "Capacity planning for agencies": Tuesday 9 December 2026, 17:00 Berlin, 60 minutes. Felix Amann, Solutions Engineer. 50 seats. Join link https://meet.kitewell.example/agencies. Description: "How agencies use the workload view to stop overbooking their people. Live demo with a real agency setup, plus Q&A."
3. "What's new in Kitewell: winter release": Wednesday 14 January 2027, 15:00 Berlin, 30 minutes. Lukas Brandt (me), Events Lead. 100 seats. Join link https://meet.kitewell.example/winter-release. Description: "A quick tour of everything new this winter, in 30 minutes, with time for questions."

**What visitors see:** only upcoming sessions, soonest first, each with date and time, length, speaker, and seats left ("12 seats left", or "Full: join the waitlist"). Our audience is mostly Germany, the UK and the US East Coast, so show the time in Berlin, London and New York. Registration closes when a session starts, and past sessions disappear from the public page by themselves.

**The registration form:** first name, last name, work email, company, job title (optional). One checkbox, NOT ticked by default: "Yes, send me Kitewell product news and event invites. You can unsubscribe at any time." Legal needs to know for every person whether they ticked it, when, and the exact sentence they saw. People with Gmail, Outlook.com, Yahoo, GMX, web.de and similar addresses can register, but flag them in my list (sales doesn't call those).

**The rules — this is where the spreadsheet failed us:**
- The same email twice for the same session: "You're already registered for this session." No second seat.
- Never more people than seats. If two people take the last seat at the same moment, only one gets it.
- When a session is full, people join the waitlist instead. First come, first served. When someone cancels, or when I add seats, the first people on the waitlist move up automatically.
- People can cancel their own seat.

**After registering:** I was told the tool can't send emails yet, so the confirmation page is all they get, and it must be their own page they can bookmark. It shows: "You're in" (or "You're number 3 on the waitlist"), the session, date and time, the Zoom join link (only for people with a seat, never on the public page: we've been Zoom-bombed before), buttons "Add to Google Calendar" and "Add to Outlook", an .ics file for Apple Calendar if that's possible, and "Can't make it? Cancel your seat". When someone on the waitlist moves up, their page shows that they're in the next time they open it. When I cancel a session, their page says it's cancelled.

**Where people come from:** our ads link to the page with `?utm_source=linkedin&utm_campaign=q4-agencies` and the like. Keep source and campaign with each registration. I want to see registrations by source for each session.

**My page** (only for the events team): for each session, registered of seats, waitlist, cancellations, and registrations by source. The attendee list: name, email, company, job title, consent (yes/no and when), registered when, source and campaign, the free-mail flag, and status (registered, waitlist, cancelled). I import attendees into HubSpot after each session: a CSV download would be perfect; if the tool can't do downloads, give me a plain table I can select and paste into a spreadsheet.

**Look:** it's for customers and prospects, so it must look great and professional. Half our registrations come from LinkedIn on phones, so phones first. Brand: Kitewell indigo #3B3FD8 for buttons and highlights, ink #171A33 for text, light backgrounds, a clean sans-serif. Our design team made the icon, it's in design/icon.svg in your folder: please use it as the app's icon.

**Where it goes:** our platform team gave me this: deploy it to http://127.0.0.1:8111 (that's where you connect), and call the app `webinars`. Put me down as the owner. When it's deployed, send me the address so I can open it.

Afterwards, write me a short note: what was unclear or annoying while building it?
