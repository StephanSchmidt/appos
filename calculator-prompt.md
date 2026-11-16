# Calculator prompt

The test for the `appos` CLI. Give it to a fresh agent in an empty directory,
with only the `appos` binary on its PATH and its own `APPOS_CONFIG`. It must
not read the AppOS source or search the web for appos. Use it word for word.

> Hi! Our platform team says we can now build our own little web apps with a
> tool called "appos". Could you build me a calculator with it? It should
> remember my recent calculations, and when I come back later it should still
> show what I last calculated. And please make it look great — modern and
> beautiful, something I'd be proud to show to the team. Leave it running and
> tell me where I can open it.
>
> Afterwards, write me a short note: what was unclear or annoying while
> building it?

Then check the app yourself: with curl (full page, `HX-Request: true`, posts
without JavaScript get 303, escaping, the server log) and with screenshots.
Fix appos (CLI, guide, shell, contract), never the agent's app.
