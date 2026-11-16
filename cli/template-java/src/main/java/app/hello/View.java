// The hello module's pages, as html templates. The page's parts that htmx
// swaps in (greeting, history, name_error) are methods of their own, used
// by the page and by the handler that updates them.

package app.hello;

import static app.Appos.EMPTY;
import static app.Appos.html;

import app.Appos.Html;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;

final class View {
    private View() {}

    /**
     * What the page shows.
     *
     * @param history Newest first; the first one is the greeting.
     * @param name What was typed into the form, and `error` why it didn't
     *     parse.
     */
    record Index(String base, List<Greeting> history, String name, Optional<NameError> error) {}

    static Html index(Index t) {
        return html("""
                <div class="hello">
                  <section class="intro">
                    <span class="eyebrow">Your new app</span>
                    <h1>Hello, <span class="gradient-text">world</span></h1>
                    <p class="lead">This is your app. Make it yours: the page is src/main/java/app/hello/View.java, the design src/main/java/app/Style.java, the logic src/main/java/app/hello/.</p>
                  </section>

                  <section class="card">
                    <h2>Say hello</h2>
                    <form class="greet" method="post" action="{}/greet" hx-post="{}/greet" hx-swap="none">
                      <input type="text" name="name" value="{}" placeholder="Your name" maxlength="{}" autocomplete="off" aria-describedby="name_error">
                      <button>Say hello</button>
                    </form>
                    <p class="error" id="name_error" aria-live="polite">{}</p>
                  </section>

                  <section class="greeting" id="greeting">{}</section>

                  <section class="card">
                    <h2>Recent hellos</h2>
                    <ul class="history" id="history">{}</ul>
                  </section>
                </div>""", t.base(), t.base(), t.name(), Name.MAX, nameError(t), greeting(t), history(t));
    }

    static Html nameError(Index t) {
        return t.error().map(e -> html("{}", e.text())).orElse(EMPTY);
    }

    static Html greeting(Index t) {
        if (t.history().isEmpty()) {
            return html("<p class=\"name\">Hello!</p>");
        }
        Greeting g = t.history().get(0);
        return html("""
                <p class="name">Hello, {}!</p>
                      <p class="when">Said hello at {}.</p>""", g.name(), g.time());
    }

    static Html history(Index t) {
        if (t.history().isEmpty()) {
            return html("<li class=\"empty\">Nobody yet. Be the first!</li>");
        }
        List<Html> items = new ArrayList<>();
        for (Greeting g : t.history()) {
            items.add(html("<li><a class=\"who\" href=\"{}/greetings/{}\">{}</a><time>{}</time></li>", t.base(), g.id(), g.name(), g.time()));
        }
        return html("{}", items);
    }

    /** One greeting's page. */
    static Html greetingPage(String base, Greeting g) {
        return html("""
                <div class="hello">
                  <section class="greeting">
                    <p class="name">Hello, {}!</p>
                    <p class="when">Said hello on {} at {}.</p>
                  </section>
                  <p class="back"><a href="{}">← All hellos</a></p>
                </div>""", g.name(), g.date(), g.time(), base);
    }
}
