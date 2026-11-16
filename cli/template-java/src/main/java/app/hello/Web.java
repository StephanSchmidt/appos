// The hello module's handlers: parse the input, call usecases with the
// parsed values, and turn what they return, errors included, into the
// page. Errors are handled here and only here (see ERRORS in appos guide).

package app.hello;

import static app.Appos.failed;
import static app.Appos.form;
import static app.Appos.isId;
import static app.Appos.notFound;
import static app.Appos.param;
import static app.Appos.style;
import static app.Appos.title;
import static app.Appos.url;

import app.Style;
import app.hello.Name.Parsed;
import app.hello.ShowUsecase.ShowResult;
import appos.wit.Wit.Page;
import appos.wit.Wit.Types.Request;
import java.util.List;
import java.util.Optional;

final class Web {
    private Web() {}

    static void index(Request req) {
        fullPage(req, "", Optional.empty());
    }

    static void greet(Request req) {
        String typed = form(req, "name").orElse("");
        switch (Name.parse(typed)) {
            case Parsed.Invalid(NameError error) -> invalid(req, typed, error);
            case Parsed.Ok(Name name) -> {
                GreetUsecase.greetUc(name, req.now());
                if (req.htmx()) {
                    // Only these parts change; the form keeps its text and focus.
                    View.Index t = new View.Index(req.base(), ListUsecase.listUc(), "", Optional.empty());
                    Page.setHtml("greeting", View.greeting(t).text());
                    Page.setHtml("history", View.history(t).text());
                    // Clears an error from an earlier try.
                    Page.setHtml("name_error", View.nameError(t).text());
                } else {
                    // Without JavaScript: back to the page, which shows the new state.
                    Page.redirect(url(req, "/"));
                }
            }
        }
    }

    static void greeting(Request req) {
        // Text that isn't an id is as unknown as an id nobody has: 404.
        Optional<String> id = param(req, "id").filter(text -> isId(text));
        if (id.isEmpty()) {
            notFound();
            return;
        }
        // Every case by name: a new kind of result doesn't compile until
        // this handler decides what the visitor sees.
        switch (ShowUsecase.showUc(id.get())) {
            case ShowResult.Shown(Greeting greeting) -> {
                style(Style.CSS);
                title("Hello, " + greeting.name());
                Page.setHtml("page", View.greetingPage(req.base(), greeting).text());
            }
            case ShowResult.NotFound() -> notFound();
            case ShowResult.Unreadable(String why) -> failed(why);
        }
    }

    /**
     * Input that doesn't parse: status 422 and the error next to the field.
     * With htmx only the error changes; without, the whole page comes back
     * with what was typed.
     */
    private static void invalid(Request req, String typed, NameError error) {
        Page.setStatus(422);
        if (req.htmx()) {
            Page.setHtml("name_error", View.nameError(new View.Index(req.base(), List.of(), "", Optional.of(error))).text());
        } else {
            fullPage(req, typed, Optional.of(error));
        }
    }

    private static void fullPage(Request req, String name, Optional<NameError> error) {
        style(Style.CSS);
        title("Hello");
        Page.setHtml("page", View.index(new View.Index(req.base(), ListUsecase.listUc(), name, error)).text());
    }
}
