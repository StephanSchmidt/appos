package app;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;

import app.Appos.Json;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import org.junit.jupiter.api.Test;

class JsonTest {
    @Test
    void parsesEveryKindOfValue() {
        Object parsed = Json.parse(" {\"a\": [1, -2.5e1, true, false, null], \"b\": \"x\\\"\\u00e9\\n\", \"c\": {}} ");
        Map<String, Object> o = Json.object(parsed);
        assertEquals(Arrays.asList(1.0, -25.0, true, false, null), Json.list(o, "a"));
        assertEquals("x\"é\n", Json.string(o, "b"));
        assertEquals(Map.of(), Json.object(o.get("c")));
        assertNull(Json.parse("null"));
    }

    @Test
    void writesWhatItParses() {
        String text = "{\"id\":\"1\",\"n\":3,\"x\":0.5,\"ok\":true,\"none\":null,\"list\":[\"a\\\\b\",\"\\u0001\"]}";
        assertEquals(text, Json.write(Json.parse(text)));
        assertEquals("{\"k\":\"v\",\"n\":1}", Json.write(Json.of("k", "v", "n", 1)));
    }

    @Test
    void refusesWhatIsntJson() {
        for (String bad : List.of("", "{", "[1,]", "{\"a\" 1}", "\"open", "tru", "01x", "1 2", "{a:1}", "\"\\x\"", "-")) {
            assertThrows(Json.Error.class, () -> Json.parse(bad), bad);
        }
        assertThrows(Json.Error.class, () -> Json.parse("[".repeat(1000)));
    }

    /** A message names an emoji whole, not half of its surrogate pair. */
    @Test
    void messagesKeepCharactersWhole() {
        assertEquals("unexpected 😀 at character 0", assertThrows(Json.Error.class, () -> Json.parse("😀😀")).getMessage());
        assertEquals("bad escape \\😀 at character 3", assertThrows(Json.Error.class, () -> Json.parse("\"\\😀\"")).getMessage());
    }

    @Test
    void readersSayWhatIsMissing() {
        Map<String, Object> o = Json.object(Json.parse("{\"n\":1}"));
        assertEquals(1.0, Json.number(o, "n"));
        assertEquals("expected text at n", assertThrows(Json.Error.class, () -> Json.string(o, "n")).getMessage());
        assertThrows(Json.Error.class, () -> Json.bool(o, "missing"));
        assertThrows(Json.Error.class, () -> Json.object(List.of()));
    }
}
