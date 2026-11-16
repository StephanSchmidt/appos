package app.hello;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertTrue;

import app.hello.Name.Parsed;
import org.junit.jupiter.api.Test;

class NameTest {
    static Name name(String input) {
        return switch (Name.parse(input)) {
            case Parsed.Ok(Name name) -> name;
            case Parsed.Invalid(NameError error) -> throw new AssertionError(error);
        };
    }

    @Test
    void namesAreTrimmed() {
        assertEquals("Ann", name("  Ann ").text());
    }

    @Test
    void noNameGreetsTheWorld() {
        assertEquals("world", name("   ").text());
    }

    @Test
    void longNamesAreRefused() {
        String max = "é".repeat(Name.MAX);
        assertEquals(max, name(max).text());
        // Characters, not UTF-16 units: an emoji is one.
        String emoji = "🙂".repeat(Name.MAX);
        assertEquals(emoji, name(emoji).text());
        Parsed parsed = Name.parse(max + "x");
        assertEquals(new Parsed.Invalid(new NameError.TooLong(Name.MAX)), parsed);
        assertTrue(((Parsed.Invalid) parsed).error().text().contains("40 characters"));
    }
}
