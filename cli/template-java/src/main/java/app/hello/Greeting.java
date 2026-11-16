package app.hello;

import app.Appos;
import app.Appos.Json;
import java.util.Map;

/**
 * One hello, as stored in kv (JSON).
 *
 * @param id From `newId`: ids sort in the order the greetings came in.
 * @param at When, on the platform's clock, as RFC 3339 text:
 *     "2026-11-11T14:05:09+01:00".
 */
public record Greeting(String id, String name, String at) {
    /** How many of the newest greetings the page shows. */
    public static final int HISTORY = 10;

    /** A new greeting, with a new id. */
    public static Greeting create(Name name, String at) {
        return new Greeting(Appos.newId(), name.text(), at);
    }

    /** The date, "2026-11-11". */
    public String date() {
        return at.substring(0, 10);
    }

    /** The time of day, "14:05". */
    public String time() {
        return at.substring(11, 16);
    }

    /** As stored in kv. */
    Map<String, Object> toJson() {
        return Json.of("id", id, "name", name, "at", at);
    }

    /** From kv: Json.Error if it isn't a greeting. */
    static Greeting fromJson(Object json) {
        Map<String, Object> o = Json.object(json);
        String at = Json.string(o, "at");
        if (at.length() < 16) {
            throw new Json.Error("expected a time at at");
        }
        return new Greeting(Json.string(o, "id"), Json.string(o, "name"), at);
    }
}
