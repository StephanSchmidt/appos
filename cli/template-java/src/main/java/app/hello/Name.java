package app.hello;

/**
 * A name to greet. Parse, don't validate: the only way to make one is
 * `Name.parse`, so code that takes a Name never sees an invalid one. Input
 * is checked once, where it comes in, and the rest of the app relies on the
 * type.
 */
public final class Name {
    /**
     * The longest name, in characters. The form says the same (maxlength),
     * so the browser stops typing there; the server checks it anyway.
     */
    public static final int MAX = 40;

    private final String text;

    private Name(String text) {
        this.text = text;
    }

    public String text() {
        return text;
    }

    /** What `parse` made of the input: a name, or why it isn't one. */
    public sealed interface Parsed {
        record Ok(Name name) implements Parsed {}

        record Invalid(NameError error) implements Parsed {}
    }

    /**
     * Parses what a visitor typed: trimmed, at most MAX characters; empty
     * means the world. A form with more fields parses each field like this,
     * into its own type.
     */
    public static Parsed parse(String input) {
        // trim, not strip: strip loads TeaVM's Unicode tables, about 1.7 ms
        // per request, since every request starts a fresh instance.
        String name = input.trim();
        if (name.codePointCount(0, name.length()) > MAX) {
            return new Parsed.Invalid(new NameError.TooLong(MAX));
        }
        return new Parsed.Ok(new Name(name.isEmpty() ? "world" : name));
    }
}
