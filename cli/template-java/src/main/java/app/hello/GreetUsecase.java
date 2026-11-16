// Usecase: greet someone.

package app.hello;

import app.Appos.Json;
import appos.wit.Wit.Log;
import java.util.function.Consumer;

final class GreetUsecase {
    private GreetUsecase() {}

    /**
     * Greets `name` at `now`, adds the greeting to the history and logs it.
     * It takes a Name, not text: the handler has checked the input already.
     */
    static Greeting greetUc(Name name, String now) {
        return greetU(name, now, Db::addGreeting, Log::info);
    }

    static Greeting greetU(Name name, String now, Consumer<Greeting> addGreeting, Consumer<String> log) {
        Greeting greeting = Greeting.create(name, now);
        addGreeting.accept(greeting);
        // JSON quotes the name, so a name with a line break can't fake a log line.
        log.accept("hello: " + Json.write(greeting.name()));
        return greeting;
    }
}
