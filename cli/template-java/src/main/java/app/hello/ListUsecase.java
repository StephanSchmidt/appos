// Usecase: list the newest greetings.

package app.hello;

import java.util.List;
import java.util.function.IntFunction;

final class ListUsecase {
    private ListUsecase() {}

    /** The newest greetings, newest first. */
    static List<Greeting> listUc() {
        return listU(Db::loadHistory);
    }

    static List<Greeting> listU(IntFunction<List<Greeting>> loadHistory) {
        return loadHistory.apply(Greeting.HISTORY);
    }
}
