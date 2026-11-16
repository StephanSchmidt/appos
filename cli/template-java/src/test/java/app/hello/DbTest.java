package app.hello;

import static org.junit.jupiter.api.Assertions.assertEquals;

import app.Appos;
import org.junit.jupiter.api.Test;

class DbTest {
    @Test
    void keysAreThePrefixAndTheId() {
        String id = Appos.newId();
        assertEquals("hello/history/" + id, Db.key(id));
    }
}
