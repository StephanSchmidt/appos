package app.hello;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import java.util.Optional;
import org.junit.jupiter.api.Test;

class ViewTest {
    static View.Index index(List<Greeting> history, String name) {
        return new View.Index("/hello", history, name, Optional.empty());
    }

    @Test
    void pageBeforeAnyHello() {
        String page = View.index(index(List.of(), "")).text();
        assertTrue(page.contains("Hello!"), page);
        assertTrue(page.contains("Nobody yet"), page);
        assertTrue(page.contains("action=\"/hello/greet\""), page);
        assertTrue(page.contains("maxlength=\"40\""), page);
    }

    @Test
    void greetingAndHistoryShowTheNewestHello() {
        Greeting greeting = GreetingTest.ann();
        View.Index t = index(List.of(greeting), "");
        assertTrue(View.greeting(t).text().contains("Hello, Ann!"));
        String history = View.history(t).text();
        assertTrue(history.contains("14:05"), history);
        assertTrue(history.contains("href=\"/hello/greetings/" + greeting.id() + "\""), history);
    }

    @Test
    void anErrorShowsNextToWhatWasTyped() {
        View.Index t = new View.Index("/hello", List.of(), "Annnn", Optional.of(new NameError.TooLong(40)));
        String page = View.index(t).text();
        assertTrue(page.contains("value=\"Annnn\""), page);
        assertTrue(page.contains("keep it to 40 characters"), page);
    }

    @Test
    void noErrorRendersNothing() {
        assertEquals("", View.nameError(index(List.of(), "")).text());
    }

    @Test
    void aGreetingsPageShowsWhen() {
        String page = View.greetingPage("/hello", GreetingTest.ann()).text();
        assertTrue(page.contains("Hello, Ann!"), page);
        assertTrue(page.contains("2026-11-11 at 14:05"), page);
    }

    @Test
    void namesAreEscaped() {
        Greeting evil = Greeting.create(NameTest.name("<b>Ann</b>"), GreetingTest.NOW);
        String page = View.index(index(List.of(evil), "<b>")).text();
        assertFalse(page.contains("<b>"), page);
        assertTrue(page.contains("&#60;b&#62;Ann"), page);
    }
}
