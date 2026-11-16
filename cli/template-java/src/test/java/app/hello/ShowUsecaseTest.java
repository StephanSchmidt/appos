package app.hello;

import static org.junit.jupiter.api.Assertions.assertEquals;

import app.Appos.Read;
import app.hello.ShowUsecase.ShowResult;
import org.junit.jupiter.api.Test;

class ShowUsecaseTest {
    @Test
    void showsTheGreeting() {
        Greeting ann = GreetingTest.ann();
        assertEquals(new ShowResult.Shown(ann), ShowUsecase.showU(ann.id(), id -> new Read.Found<>(ann)));
    }

    @Test
    void anUnknownIdIsNotFound() {
        assertEquals(new ShowResult.NotFound(), ShowUsecase.showU("an-id", id -> new Read.Missing<>()));
    }

    @Test
    void aGreetingThatCantBeReadIsAnError() {
        String why = "hello/history/1 is unreadable: unexpected x at character 0";
        assertEquals(new ShowResult.Unreadable(why), ShowUsecase.showU("an-id", id -> new Read.Unreadable<>(why)));
    }
}
