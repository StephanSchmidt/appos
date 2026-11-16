// Hello world: an AppOS app in Java. Change it into yours; `appos guide`
// explains the shell and its calls.
//
// An app is made of modules, one package each under src/main/java/app/.
// This one has a single module, hello. A module owns its routes, its pages
// and its kv keys, and every module has the same layers:
//   Hello.java          its routes and menu entries (MODULE)
//   Web.java            handlers: parse the input (422 and the message if
//                       it doesn't parse), call usecases with the parsed
//                       values, turn their results and errors (an unknown
//                       id: 404) into the page. Errors end here.
//   View.java           the page and its parts, as html templates
//   <Name>Usecase.java  one usecase per file, as a pair:
//                         greetU    the logic; every IO it needs comes in
//                                   as a parameter (addGreeting, log), so
//                                   tests pass fakes
//                         greetUc   wires greetU to the real IO in Db.java
//                       A usecase that can fail says how in its result
//                       (ShowUsecase.java: ShowResult.NotFound).
//   Db.java             the IO: every kv call, with the keys; package-
//                       private, so other modules can't reach the data
//   Name.java,          the data and its rules, without IO; also the checks
//   Greeting.java       of what visitors type (Name.parse)
// Tests sit in src/test/java, in the same package; `mvn test` runs them on
// the JVM, without the shell (make check builds and tests).
//
// Appos.java is the platform's side: leave it to `appos sync`.

package app;

import app.Appos.Module;
import app.hello.Hello;
import appos.wit.Wit;
import appos.wit.Wit.Page;
import appos.wit.Wit.Types.Request;
import java.util.List;

public final class App implements Wit.Handler {
    /** Every module of the app: a new module adds its MODULE here. */
    static final List<Module> MODULES = List.of(Hello.MODULE);

    @Override
    public void init() {
        Appos.register(MODULES);
    }

    @Override
    public void start(String handler, Request req) {
        Appos.dispatch(MODULES, handler, req);
        Page.done();
    }
}
