package app.hello;

/**
 * Why text is not a Name. Code and tests look at the record; `text` is the
 * message for the visitor.
 */
public sealed interface NameError {
    record TooLong(int max) implements NameError {}

    default String text() {
        return switch (this) {
            case TooLong(int max) -> "That's a long name! Please keep it to " + max + " characters.";
        };
    }
}
