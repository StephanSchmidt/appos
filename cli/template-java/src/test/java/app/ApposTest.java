package app;

import static app.Appos.html;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertFalse;
import static org.junit.jupiter.api.Assertions.assertThrows;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import org.junit.jupiter.api.Test;

class ApposTest {
    @Test
    void templatesEscapeTextButNotHtml() {
        assertEquals("<p>&#60;b&#62; &#38; &#34;&#39; 3</p>", html("<p>{} {}</p>", "<b> & \"'", 3).text());
        assertEquals("<ul><li>a</li><li>&#60;</li></ul>", html("<ul>{}</ul>", List.of(html("<li>{}</li>", "a"), html("<li>{}</li>", "<"))).text());
        assertEquals("<i>x</i>", html("{}", Appos.raw("<i>x</i>")).text());
    }

    @Test
    void templatesCountTheirValues() {
        assertThrows(IllegalArgumentException.class, () -> html("<p>{}</p>"));
        assertThrows(IllegalArgumentException.class, () -> html("<p></p>", "x"));
    }

    @Test
    void valuesAreEncoded() {
        assertEquals("a%20b%2F%C3%A9-_.~", Appos.encode("a b/é-_.~"));
    }

    @Test
    void idsSortInTheOrderTheyWereMade() {
        String last = Appos.newId();
        for (int i = 0; i < 1000; i++) {
            String id = Appos.newId();
            assertTrue(Appos.isId(id), id);
            assertEquals('7', id.charAt(14), id);
            assertTrue(last.compareTo(id) < 0, last + " " + id);
            last = id;
        }
    }

    @Test
    void onlyIdsAreIds() {
        assertFalse(Appos.isId("nope"));
        assertFalse(Appos.isId("01A1E001-64F8-7E8D-811F-6E948D79B9B7"));
        assertFalse(Appos.isId("01a1e001x64f8-7e8d-811f-6e948d79b9b7"));
    }
}
