package app.hello;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import app.Appos.Json;
import org.junit.jupiter.api.Test;

class GreetingTest {
    static final String NOW = "2026-11-11T14:05:09+01:00";

    static Greeting ann() {
        return Greeting.create(NameTest.name("Ann"), NOW);
    }

    @Test
    void aGreetingKnowsItsDateAndTime() {
        Greeting greeting = ann();
        assertEquals("Ann", greeting.name());
        assertEquals("2026-11-11", greeting.date());
        assertEquals("14:05", greeting.time());
    }

    @Test
    void greetingsMadeOneAfterTheOtherSortInThatOrder() {
        assertTrue(ann().id().compareTo(ann().id()) < 0);
    }

    @Test
    void greetingsGoThroughJson() {
        Greeting greeting = ann();
        assertEquals(greeting, Greeting.fromJson(Json.parse(Json.write(greeting.toJson()))));
    }

    @Test
    void jsonThatIsNoGreetingIsRefused() {
        assertThrows(Json.Error.class, () -> Greeting.fromJson(Json.parse("[]")));
        assertThrows(Json.Error.class, () -> Greeting.fromJson(Json.parse("{\"id\":\"1\",\"name\":\"Ann\"}")));
        assertThrows(Json.Error.class, () -> Greeting.fromJson(Json.parse("{\"id\":\"1\",\"name\":\"Ann\",\"at\":\"today\"}")));
    }
}
