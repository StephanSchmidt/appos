package app.hello;

import static org.junit.jupiter.api.Assertions.assertEquals;

import java.util.ArrayList;
import java.util.List;
import org.junit.jupiter.api.Test;

class GreetUsecaseTest {
    @Test
    void greetsAddsAndLogs() {
        List<String> added = new ArrayList<>();
        List<String> logged = new ArrayList<>();

        Greeting greeting = GreetUsecase.greetU(NameTest.name("Ann"), GreetingTest.NOW, g -> added.add(g.name()), logged::add);

        assertEquals("Ann", greeting.name());
        assertEquals(List.of("Ann"), added);
        assertEquals(List.of("hello: \"Ann\""), logged);
    }

    @Test
    void aLineBreakCantFakeALogLine() {
        List<String> logged = new ArrayList<>();
        GreetUsecase.greetU(NameTest.name("Ann\nhello: \"Bob\""), GreetingTest.NOW, g -> {}, logged::add);
        assertEquals(List.of("hello: \"Ann\\nhello: \\\"Bob\\\"\""), logged);
    }
}
