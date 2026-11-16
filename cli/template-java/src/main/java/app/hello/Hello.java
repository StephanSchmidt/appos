// The hello module: say hello, and see who said hello before. Other modules
// reach it only through this class; its data (Db.java) stays private to it.

package app.hello;

import static app.Appos.get;
import static app.Appos.post;

import app.Appos.Menu;
import app.Appos.Module;
import java.util.List;

public final class Hello {
    private Hello() {}

    /**
     * The module's routes and menu entries. Paths are below the app's base;
     * `:name` segments arrive as `param(req, "name")`.
     */
    public static final Module MODULE = new Module(
            List.of(get("/", Web::index), post("/greet", Web::greet), get("/greetings/:id", Web::greeting)),
            List.of(new Menu("Hello", "/")));
}
