// The app's side of AppOS: the bindings `appos build` generates from wit/
// (the shell's contract, appos.wit.Wit in bindings/) plus helpers: routes
// and modules, links, HTML templates, JSON, kv with JSON, ids. `appos new`
// writes this file and `appos sync` brings it up to date with the `appos`
// you have, so keep your own code elsewhere.
//
// An app has two functions. `init` runs once and registers its modules'
// routes and menu entries (`register(MODULES)`). `start` runs per request
// (`dispatch(MODULES, ...)`): it sets named HTML with `Page.setHtml`
// ("page" for the whole page, any other name for an htmx update of the
// element with that id) and calls `Page.done()`.
//
// HTML comes from `html` templates: html("<p>{}</p>", text) escapes every
// value, unless it is already Html (another template, a list of them, or
// `raw`).

package app;

import appos.wit.Wit.Kv;
import appos.wit.Wit.Log;
import appos.wit.Wit.Page;
import appos.wit.Wit.Registry;
import appos.wit.Wit.Tuple2;
import appos.wit.Wit.Types.Request;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.function.Consumer;
import java.util.function.Function;

public final class Appos {
    private Appos() {}

    // Routes and modules

    /**
     * A route: method, path and the function that handles it. `path` is
     * below the app's base; `:name` segments arrive as `param(req, "name")`.
     */
    public record Route(String method, String path, Consumer<Request> handler) {}

    public static Route get(String path, Consumer<Request> handler) {
        return new Route("GET", path, handler);
    }

    public static Route post(String path, Consumer<Request> handler) {
        return new Route("POST", path, handler);
    }

    /** A menu entry: its label and path. */
    public record Menu(String label, String path) {}

    /**
     * A part of the app: its routes and its menu entries. The app lists its
     * modules once, `List.of(Hello.MODULE)`, for both `register` and
     * `dispatch`.
     */
    public record Module(List<Route> routes, List<Menu> menu) {}

    /** The name the shell hands to `start`, e.g. "POST /greet": unique as long as method and path are. */
    private static String routeName(Route route) {
        return route.method() + " " + route.path();
    }

    /** Registers every module's routes and menu entries, during `init`. */
    public static void register(List<Module> modules) {
        for (Module module : modules) {
            for (Route route : module.routes()) {
                Registry.addRoute(route.method(), route.path(), routeName(route));
            }
            for (Menu entry : module.menu()) {
                Registry.addMenu(entry.label(), entry.path());
            }
        }
    }

    /**
     * Runs the route the shell picked for this request, from all modules'
     * routes; a 404 page if there is none. An exception goes to the log and
     * stops the request: the shell answers 500 and keeps none of its kv
     * writes.
     */
    public static void dispatch(List<Module> modules, String handler, Request req) {
        for (Module module : modules) {
            for (Route route : module.routes()) {
                if (routeName(route).equals(handler)) {
                    try {
                        route.handler().accept(req);
                    } catch (RuntimeException e) {
                        Log.warn(handler + ": " + e);
                        throw e;
                    }
                    return;
                }
            }
        }
        notFound();
    }

    // The request

    private static Optional<String> find(List<Tuple2<String, String>> pairs, String key) {
        for (Tuple2<String, String> pair : pairs) {
            if (pair.f0().equals(key)) {
                return Optional.of(pair.f1());
            }
        }
        return Optional.empty();
    }

    /** A route parameter, e.g. `:id`. */
    public static Optional<String> param(Request req, String name) {
        return find(req.params(), name);
    }

    /** A query parameter. */
    public static Optional<String> query(Request req, String name) {
        return find(req.query(), name);
    }

    /** A form field of a POST. */
    public static Optional<String> form(Request req, String name) {
        return find(req.form(), name);
    }

    /** A link to a path inside this app: url(req, "/desk") is "/events/desk", url(req, "/") "/events". */
    public static String url(Request req, String path) {
        String rest = path.startsWith("/") ? path.substring(1) : path;
        String separator = rest.isEmpty() || rest.startsWith("?") ? "" : "/";
        return req.base() + separator + rest;
    }

    /** A link with a query string, values encoded: urlWith(req, "/", "q", "a b") is "/events?q=a%20b". */
    public static String urlWith(Request req, String path, String... keysAndValues) {
        StringBuilder out = new StringBuilder(url(req, path));
        for (int i = 0; i + 1 < keysAndValues.length; i += 2) {
            out.append(i == 0 ? '?' : '&').append(encode(keysAndValues[i])).append('=').append(encode(keysAndValues[i + 1]));
        }
        return out.toString();
    }

    /** The same link with the site's origin, e.g. for QR codes and emails. */
    public static String absoluteUrl(Request req, String path) {
        return req.origin() + url(req, path);
    }

    private static final String HEX = "0123456789ABCDEF";

    /** Percent-encodes a query value: everything but A-Z a-z 0-9 - _ . ~ */
    public static String encode(String value) {
        StringBuilder out = new StringBuilder();
        for (byte b : value.getBytes(StandardCharsets.UTF_8)) {
            int c = b & 0xff;
            boolean unreserved = (c >= '0' && c <= '9') || (c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '-' || c == '_' || c == '.' || c == '~';
            if (unreserved) {
                out.append((char) c);
            } else {
                out.append('%').append(HEX.charAt(c >> 4)).append(HEX.charAt(c & 15));
            }
        }
        return out.toString();
    }

    // HTML

    /** HTML-escapes text. */
    public static String escape(String s) {
        StringBuilder out = new StringBuilder(s.length());
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '&' -> out.append("&#38;");
                case '<' -> out.append("&#60;");
                case '>' -> out.append("&#62;");
                case '"' -> out.append("&#34;");
                case '\'' -> out.append("&#39;");
                default -> out.append(c);
            }
        }
        return out.toString();
    }

    /** HTML that is safe to send as is. */
    public record Html(String text) {}

    /**
     * A template: html("<p>{}</p>", text). Each {} takes the next value:
     * text and numbers are escaped; Html (another template, a list of them,
     * `raw`) is inserted as is.
     */
    public static Html html(String template, Object... values) {
        StringBuilder out = new StringBuilder(template.length());
        int from = 0;
        for (Object value : values) {
            int at = template.indexOf("{}", from);
            if (at < 0) {
                throw new IllegalArgumentException("more values than {} in the template");
            }
            out.append(template, from, at);
            append(out, value);
            from = at + 2;
        }
        if (template.indexOf("{}", from) >= 0) {
            throw new IllegalArgumentException("more {} in the template than values");
        }
        out.append(template, from, template.length());
        return new Html(out.toString());
    }

    private static void append(StringBuilder out, Object value) {
        if (value instanceof Html h) {
            out.append(h.text());
        } else if (value instanceof List<?> list) {
            for (Object item : list) {
                append(out, item);
            }
        } else {
            out.append(escape(String.valueOf(value)));
        }
    }

    /** Markup that is already HTML, inserted unescaped. */
    public static Html raw(String text) {
        return new Html(text);
    }

    /** Nothing: for the empty branch of `cond ? html(...) : EMPTY`. */
    public static final Html EMPTY = new Html("");

    /** Adds CSS to the page's head, e.g. the app's design. */
    public static void style(String css) {
        Page.addHeader("<style>" + css + "</style>");
    }

    /** Sets the page's title, escaped like the Rust SDK's (&amp;). */
    public static void title(String text) {
        String escaped = text.replace("&", "&amp;").replace("<", "&lt;").replace(">", "&gt;").replace("\"", "&quot;").replace("'", "&#39;");
        Page.addHeader("<title>" + escaped + "</title>");
    }

    /**
     * Status 500 and a page saying the page can't be shown; `why` goes to the
     * log, not the page. For what the app can't work with, like stored data
     * that isn't JSON (Read.Unreadable).
     */
    public static void failed(String why) {
        Log.warn(why);
        Page.setStatus(500);
        title("Can't be shown");
        Page.setHtml("page", "<h1 style=\"font-family: system-ui; text-align: center; margin: 4rem 1rem\">This page can't be shown right now</h1>");
    }

    /** A 404 page. */
    public static void notFound() {
        Page.setStatus(404);
        title("Not found");
        Page.setHtml("page", "<h1 style=\"font-family: system-ui; text-align: center; margin: 4rem 1rem\">Not found</h1>");
    }

    // JSON

    /**
     * JSON for kv. Parsed JSON is plain Java: Map<String, Object> (in key
     * order), List<Object>, String, Double, Boolean, null. A record writes
     * itself with `Json.of` and reads itself with `object`, `string` and the
     * like, which throw Json.Error for JSON without what the app expects, as
     * `parse` does for text that isn't JSON.
     */
    public static final class Json {
        private Json() {}

        /** Text that isn't JSON, or JSON that isn't what the app expects. */
        public static final class Error extends RuntimeException {
            public Error(String message) {
                super(message);
            }
        }

        /** An object from keys and values: Json.of("id", id, "name", name). */
        public static Map<String, Object> of(Object... keysAndValues) {
            Map<String, Object> out = new LinkedHashMap<>();
            for (int i = 0; i + 1 < keysAndValues.length; i += 2) {
                out.put((String) keysAndValues[i], keysAndValues[i + 1]);
            }
            return out;
        }

        // Reading parsed JSON

        /** `value` as an object. */
        @SuppressWarnings("unchecked")
        public static Map<String, Object> object(Object value) {
            if (value instanceof Map<?, ?> map) {
                return (Map<String, Object>) map;
            }
            throw new Error("expected an object");
        }

        /** The text at `key`. */
        public static String string(Map<String, Object> object, String key) {
            if (object.get(key) instanceof String s) {
                return s;
            }
            throw new Error("expected text at " + key);
        }

        /** The number at `key`. */
        public static double number(Map<String, Object> object, String key) {
            if (object.get(key) instanceof Double d) {
                return d;
            }
            throw new Error("expected a number at " + key);
        }

        /** The boolean at `key`. */
        public static boolean bool(Map<String, Object> object, String key) {
            if (object.get(key) instanceof Boolean b) {
                return b;
            }
            throw new Error("expected true or false at " + key);
        }

        /** The list at `key`. */
        @SuppressWarnings("unchecked")
        public static List<Object> list(Map<String, Object> object, String key) {
            if (object.get(key) instanceof List<?> list) {
                return (List<Object>) list;
            }
            throw new Error("expected a list at " + key);
        }

        // Writing

        /** `value` as JSON text: maps, lists, strings, numbers, booleans and null. */
        public static String write(Object value) {
            StringBuilder out = new StringBuilder();
            write(out, value);
            return out.toString();
        }

        private static void write(StringBuilder out, Object value) {
            switch (value) {
                case null -> out.append("null");
                case String s -> quote(out, s);
                case Boolean b -> out.append(b);
                case Double d when d == Math.rint(d) && Math.abs(d) < 1e15 -> out.append(d.longValue());
                case Float f when f == Math.rint(f) && Math.abs(f) < 1e7 -> out.append(f.longValue());
                case Number n -> out.append(n);
                case Map<?, ?> map -> {
                    out.append('{');
                    boolean first = true;
                    for (Map.Entry<?, ?> e : map.entrySet()) {
                        if (!first) {
                            out.append(',');
                        }
                        first = false;
                        quote(out, String.valueOf(e.getKey()));
                        out.append(':');
                        write(out, e.getValue());
                    }
                    out.append('}');
                }
                case List<?> list -> {
                    out.append('[');
                    for (int i = 0; i < list.size(); i++) {
                        if (i > 0) {
                            out.append(',');
                        }
                        write(out, list.get(i));
                    }
                    out.append(']');
                }
                default -> throw new Error("can't write " + value.getClass().getName() + " as JSON");
            }
        }

        private static void quote(StringBuilder out, String s) {
            out.append('"');
            for (int i = 0; i < s.length(); i++) {
                char c = s.charAt(i);
                switch (c) {
                    case '"' -> out.append("\\\"");
                    case '\\' -> out.append("\\\\");
                    case '\n' -> out.append("\\n");
                    case '\r' -> out.append("\\r");
                    case '\t' -> out.append("\\t");
                    default -> {
                        if (c < 0x20) {
                            out.append("\\u00").append(Character.forDigit(c >> 4, 16)).append(Character.forDigit(c & 15, 16));
                        } else {
                            out.append(c);
                        }
                    }
                }
            }
            out.append('"');
        }

        // Parsing

        /** The JSON value in `text`. */
        public static Object parse(String text) {
            Parser p = new Parser(text);
            p.space();
            Object value = p.value(0);
            p.space();
            if (p.at < text.length()) {
                throw p.error("text after the value");
            }
            return value;
        }

        private static final class Parser {
            /** Deeper nesting than this is an error, not a stack overflow. */
            private static final int DEPTH = 100;

            private final String s;
            private int at;

            Parser(String s) {
                this.s = s;
            }

            Error error(String what) {
                return new Error(what + " at character " + at);
            }

            void space() {
                while (at < s.length() && " \t\n\r".indexOf(s.charAt(at)) >= 0) {
                    at++;
                }
            }

            boolean eat(char c) {
                if (at < s.length() && s.charAt(at) == c) {
                    at++;
                    return true;
                }
                return false;
            }

            void expect(char c) {
                if (!eat(c)) {
                    throw error("expected " + c);
                }
            }

            /** The character at `i` for a message: both halves of a surrogate pair. */
            String whole(int i) {
                return s.substring(i, i + Character.charCount(s.codePointAt(i)));
            }

            Object value(int depth) {
                if (depth > DEPTH) {
                    throw error("nested too deep");
                }
                if (at >= s.length()) {
                    throw error("unexpected end");
                }
                char c = s.charAt(at);
                return switch (c) {
                    case '{' -> object(depth);
                    case '[' -> array(depth);
                    case '"' -> string();
                    case 't' -> word("true", Boolean.TRUE);
                    case 'f' -> word("false", Boolean.FALSE);
                    case 'n' -> word("null", null);
                    default -> {
                        if (c == '-' || (c >= '0' && c <= '9')) {
                            yield number();
                        }
                        throw error("unexpected " + whole(at));
                    }
                };
            }

            Object word(String word, Object value) {
                if (!s.startsWith(word, at)) {
                    throw error("expected " + word);
                }
                at += word.length();
                return value;
            }

            Map<String, Object> object(int depth) {
                expect('{');
                Map<String, Object> out = new LinkedHashMap<>();
                space();
                if (eat('}')) {
                    return out;
                }
                do {
                    space();
                    if (at >= s.length() || s.charAt(at) != '"') {
                        throw error("expected a key");
                    }
                    String key = string();
                    space();
                    expect(':');
                    space();
                    out.put(key, value(depth + 1));
                    space();
                } while (eat(','));
                expect('}');
                return out;
            }

            List<Object> array(int depth) {
                expect('[');
                List<Object> out = new ArrayList<>();
                space();
                if (eat(']')) {
                    return out;
                }
                do {
                    space();
                    out.add(value(depth + 1));
                    space();
                } while (eat(','));
                expect(']');
                return out;
            }

            String string() {
                expect('"');
                StringBuilder out = new StringBuilder();
                while (true) {
                    if (at >= s.length()) {
                        throw error("unterminated text");
                    }
                    char c = s.charAt(at++);
                    if (c == '"') {
                        return out.toString();
                    }
                    if (c < 0x20) {
                        throw error("control character in text");
                    }
                    if (c != '\\') {
                        out.append(c);
                        continue;
                    }
                    if (at >= s.length()) {
                        throw error("unterminated text");
                    }
                    char e = s.charAt(at++);
                    switch (e) {
                        case '"', '\\', '/' -> out.append(e);
                        case 'b' -> out.append('\b');
                        case 'f' -> out.append('\f');
                        case 'n' -> out.append('\n');
                        case 'r' -> out.append('\r');
                        case 't' -> out.append('\t');
                        case 'u' -> {
                            if (at + 4 > s.length()) {
                                throw error("short \\u escape");
                            }
                            int code = 0;
                            for (int i = 0; i < 4; i++) {
                                int d = Character.digit(s.charAt(at++), 16);
                                if (d < 0) {
                                    throw error("bad \\u escape");
                                }
                                code = code * 16 + d;
                            }
                            out.append((char) code);
                        }
                        default -> throw error("bad escape \\" + whole(at - 1));
                    }
                }
            }

            Double number() {
                int from = at;
                eat('-');
                if (!digits()) {
                    throw error("expected a digit");
                }
                if (eat('.') && !digits()) {
                    throw error("expected a digit");
                }
                if (eat('e') || eat('E')) {
                    if (!eat('+')) {
                        eat('-');
                    }
                    if (!digits()) {
                        throw error("expected a digit");
                    }
                }
                try {
                    return Double.parseDouble(s.substring(from, at));
                } catch (NumberFormatException e) {
                    throw error("bad number");
                }
            }

            boolean digits() {
                int from = at;
                while (at < s.length() && s.charAt(at) >= '0' && s.charAt(at) <= '9') {
                    at++;
                }
                return at > from;
            }
        }
    }

    // kv with JSON

    /** The most `scanJson` returns in one call. */
    public static final int PAGE = 100;

    /**
     * What `getJson` read: the value, nothing, or stored text the app can't
     * read. Stored data can be anything: another version of the app, a
     * person or a bug may have written it. A usecase returns Unreadable as it
     * is, and the handler answers with `failed`.
     */
    public sealed interface Read<T> {
        record Found<T>(T value) implements Read<T> {}

        record Missing<T>() implements Read<T> {}

        record Unreadable<T>(String why) implements Read<T> {}
    }

    /** The value at `key`, parsed from JSON by `from`; Unreadable if it isn't JSON or `from` rejects it. */
    public static <T> Read<T> getJson(String key, Function<Object, T> from) {
        Optional<String> text = Kv.get(key);
        if (text.isEmpty()) {
            return new Read.Missing<>();
        }
        try {
            return new Read.Found<>(from.apply(Json.parse(text.get())));
        } catch (Json.Error e) {
            return new Read.Unreadable<>(key + " is unreadable: " + e.getMessage());
        }
    }

    /** Stores `json` (maps, lists, strings, numbers, booleans) at `key` as JSON. */
    public static void setJson(String key, Object json) {
        Kv.set(key, Json.write(json));
    }

    /** A key and its value. */
    public record Entry<T>(String key, T value) {}

    /**
     * Keys starting with `prefix` and their values parsed from JSON, in key
     * order (newest first with `reverse` when keys are ids from `newId`), at
     * most `limit`; pass the last key as `after` for the next page. A value
     * that isn't JSON is logged and left out, so one bad entry doesn't take
     * the whole list down.
     */
    public static <T> List<Entry<T>> scanJson(String prefix, Optional<String> after, int limit, boolean reverse, Function<Object, T> from) {
        List<Entry<T>> out = new ArrayList<>();
        for (Tuple2<String, String> pair : Kv.scan(prefix, after, limit, reverse)) {
            try {
                out.add(new Entry<>(pair.f0(), from.apply(Json.parse(pair.f1()))));
            } catch (Json.Error e) {
                Log.warn(pair.f0() + " is unreadable, left out: " + e.getMessage());
            }
        }
        return out;
    }

    /** Every key starting with `prefix`, page by page. */
    public static <T> List<Entry<T>> scanAllJson(String prefix, boolean reverse, Function<Object, T> from) {
        List<Entry<T>> out = new ArrayList<>();
        Optional<String> after = Optional.empty();
        while (true) {
            List<Tuple2<String, String>> page = Kv.scan(prefix, after, PAGE, reverse);
            for (Tuple2<String, String> pair : page) {
                try {
                    out.add(new Entry<>(pair.f0(), from.apply(Json.parse(pair.f1()))));
                } catch (Json.Error e) {
                    Log.warn(pair.f0() + " is unreadable, left out: " + e.getMessage());
                }
            }
            if (page.size() < PAGE) {
                return out;
            }
            after = Optional.of(page.get(page.size() - 1).f0());
        }
    }

    // Ids

    private static final String HEX_LOWER = "0123456789abcdef";
    private static byte[] lastId = new byte[16];
    private static long lastMs = -1;

    /**
     * A new id: a UUID v7, "019a6f3c-...". It starts with the time, so ids
     * sort in the order they were made, also within one millisecond; as kv
     * keys, a scan lists them oldest first (newest first with `reverse`).
     */
    public static String newId() {
        long ms = System.currentTimeMillis();
        byte[] b = new byte[16];
        if (ms <= lastMs) {
            // The same millisecond: the last id plus one.
            System.arraycopy(lastId, 0, b, 0, 16);
            for (int i = 15; i >= 9; i--) {
                b[i]++;
                if (b[i] != 0) {
                    break;
                }
            }
        } else {
            for (int i = 0; i < 16; i++) {
                b[i] = (byte) (Math.random() * 256);
            }
            long t = ms;
            for (int i = 5; i >= 0; i--) {
                b[i] = (byte) t;
                t >>>= 8;
            }
            b[6] = (byte) ((b[6] & 0x0f) | 0x70); // version 7
            b[8] = (byte) ((b[8] & 0x3f) | 0x80); // variant
            b[9] = (byte) (b[9] & 0x7f); // room to count up within the millisecond
            lastMs = ms;
        }
        lastId = b;
        StringBuilder s = new StringBuilder(36);
        for (int i = 0; i < 16; i++) {
            if (i == 4 || i == 6 || i == 8 || i == 10) {
                s.append('-');
            }
            s.append(HEX_LOWER.charAt((b[i] >> 4) & 15)).append(HEX_LOWER.charAt(b[i] & 15));
        }
        return s.toString();
    }

    /** True if `text` is an id as `newId` makes them (any UUID, lowercase). */
    public static boolean isId(String text) {
        if (text.length() != 36) {
            return false;
        }
        for (int i = 0; i < 36; i++) {
            char c = text.charAt(i);
            boolean dash = i == 8 || i == 13 || i == 18 || i == 23;
            if (dash ? c != '-' : HEX_LOWER.indexOf(c) < 0) {
                return false;
            }
        }
        return true;
    }
}
