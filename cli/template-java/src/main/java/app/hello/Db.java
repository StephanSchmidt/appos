// The hello module's IO: every kv call, with the keys. Usecases get these
// methods as parameters; nothing else touches kv. All keys start with
// "hello/", so they can't collide with another module's. The class is
// package-private: other modules can't reach the data.

package app.hello;

import app.Appos;
import app.Appos.Entry;
import app.Appos.Read;
import java.util.ArrayList;
import java.util.List;
import java.util.Optional;

final class Db {
    private Db() {}

    private static final String HISTORY = "hello/history/";

    /**
     * Adds a greeting to the history under a key of its own. Requests that
     * greet at the same time write different keys, so neither has to run
     * again (see STORAGE in appos guide). A count of all hellos in one key
     * would be written by every greeting.
     */
    static void addGreeting(Greeting greeting) {
        Appos.setJson(key(greeting.id()), greeting.toJson());
    }

    /**
     * The greeting with this id, if there is one; Unreadable if what is
     * stored there isn't a greeting (see ERRORS in appos guide).
     */
    static Read<Greeting> loadGreeting(String id) {
        return Appos.getJson(key(id), Greeting::fromJson);
    }

    /**
     * The newest `n` greetings, newest first: a reverse scan starts at the
     * last key. One scan reads them all; one get each would cost more of the
     * request's budget.
     */
    static List<Greeting> loadHistory(int n) {
        List<Greeting> out = new ArrayList<>();
        for (Entry<Greeting> entry : Appos.scanJson(HISTORY, Optional.empty(), n, true, Greeting::fromJson)) {
            out.add(entry.value());
        }
        return out;
    }

    /**
     * Ids sort by time, so the key order (what a scan returns) is the order
     * the greetings came in.
     */
    static String key(String id) {
        return HISTORY + id;
    }
}
