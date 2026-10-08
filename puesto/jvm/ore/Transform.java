package ore;

import java.lang.annotation.Documented;
import java.lang.annotation.ElementType;
import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;
import java.lang.annotation.Target;

/**
 * <b>A transform</b> (ORE 0055, OOS v1alpha25 {@code 01} §5.5): a {@code public static}
 * method with no parameters of the file's class (the top-level one named like the
 * {@code .java}) that produces a dataset or a media collection.
 *
 * <p>What it reads and what it writes are read <b>without compiling</b>: {@code inputs}
 * and {@code output} are string literals, or {@code static final String} fields of the
 * class initialized with one, by their name or the class's. Anything computed —also a
 * concatenation— is rejected at commit with its line. On commit the platform derives
 * the transform's document; Build calls the method, and while it runs only
 * {@code inputs} can be read and only {@code output} written.
 *
 * <pre>{@code
 * import static ore.Ore.*;
 *
 * import ore.Transform;
 *
 * public class Summary {
 *     static final String ORDERS = "sales.orders";
 *
 *     // The first line of its Javadoc is the transform's description.
 *     @Transform(inputs = {ORDERS, "sales.customers"}, output = "sales.summary")
 *     public static Object summary() throws Exception {
 *         return write("sales.summary", over(ORDERS));
 *     }
 * }
 * }</pre>
 */
@Documented
@Retention(RetentionPolicy.RUNTIME)
@Target(ElementType.METHOD)
public @interface Transform {
    /**
     * What it reads, in order: {@code <database>.<schema>.<name>} (or
     * {@code <database>.<name>}, in {@code default}). May be empty: {@code {}}.
     *
     * @return the names it reads
     */
    String[] inputs();

    /**
     * What it writes: a written dataset or a written media collection, or a name that does
     * not exist yet (the first build registers it).
     *
     * @return the name it writes
     */
    String output();
}
