// Usecase: show one greeting.

package app.hello;

import app.Appos.Read;
import java.util.function.Function;

final class ShowUsecase {
    private ShowUsecase() {}

    /**
     * The greeting, or why it can't be shown. Each usecase lists what can go
     * wrong here and nothing else, and the handler handles every case. These
     * are outcomes the data decides; input that doesn't parse never gets
     * this far.
     */
    sealed interface ShowResult {
        record Shown(Greeting greeting) implements ShowResult {}

        record NotFound() implements ShowResult {}

        record Unreadable(String why) implements ShowResult {}
    }

    static ShowResult showUc(String id) {
        return showU(id, Db::loadGreeting);
    }

    static ShowResult showU(String id, Function<String, Read<Greeting>> loadGreeting) {
        return switch (loadGreeting.apply(id)) {
            case Read.Found<Greeting>(Greeting greeting) -> new ShowResult.Shown(greeting);
            case Read.Missing<Greeting>() -> new ShowResult.NotFound();
            case Read.Unreadable<Greeting>(String why) -> new ShowResult.Unreadable(why);
        };
    }
}
