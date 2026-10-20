// ORE 0068 C3·b · EL `LEFT JOIN` ESPACIAL, CON ÍNDICE.
//
// Sedona sólo indexa un join externo con predicado espacial si el lado que NO se conserva va difundido (su
// documentación, Optimizer: «Left outer - broadcast right»). Si no, Spark lo planifica como
// `BroadcastNestedLoopJoin`: cada fila contra cada fila. Medido en victor (C3 previo): Q10 de SpatialBench
// —cada zona con sus viajes, también las que no tienen— con 20 M puntos y 22 k zonas pasa de 300 s.
//
// Esta regla lo reescribe a la forma que documenta Sedona: un join INTERNO espacial (que sí indexa) que
// devuelve el número de la fila conservada, y un join externo por ese número (de igualdad). Medido igual:
// 15,5 s con un id que ya existe y 33,1 s con uno numerado, con las mismas cuentas (`c3/b/formas.py`).
//
//   L LEFT JOIN R ON p     →   WITH c AS (SELECT L.*, monotonically_increasing_id() k FROM L)
//                              SELECT L.*, R.* FROM c LEFT JOIN
//                                (SELECT c2.k, R.* FROM c c2 JOIN R ON p[L := c2]) d ON c.k = d.k
//
// El lado conservado va en un CTE: Spark no copia en línea un CTE no determinista que se usa dos veces, lo
// calcula UNA vez y lo reutiliza, así que el número sale igual en las dos referencias. Se aplica en el
// analizador (post-hoc), donde los CTE aún existen; el optimizador los resuelve después.
//
// No se toca:
//   - un join externo cuyo lado a difundir ya va marcado (C3·a: `over()` marca lo pequeño): Sedona lo indexa;
//   - un join externo sin predicado espacial, o uno completo (FULL OUTER).
//
// La compila la etapa `puesto-spark` del Dockerfile contra los jars de Spark, y la registra
// `spark.sql.extensions` (`spark-defaults.conf`).
package ore.spark;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import org.apache.spark.sql.SparkSession;
import org.apache.spark.sql.SparkSessionExtensions;
import org.apache.spark.sql.catalyst.expressions.Alias;
import org.apache.spark.sql.catalyst.expressions.And;
import org.apache.spark.sql.catalyst.expressions.Attribute;
import org.apache.spark.sql.catalyst.expressions.AttributeSet;
import org.apache.spark.sql.catalyst.expressions.EqualTo;
import org.apache.spark.sql.catalyst.expressions.Expression;
import org.apache.spark.sql.catalyst.expressions.MonotonicallyIncreasingID;
import org.apache.spark.sql.catalyst.expressions.NamedExpression;
import org.apache.spark.sql.catalyst.expressions.NamedExpression$;
import org.apache.spark.sql.catalyst.expressions.ScalarSubquery;
import org.apache.spark.sql.catalyst.plans.Inner$;
import org.apache.spark.sql.catalyst.plans.JoinType;
import org.apache.spark.sql.catalyst.plans.LeftOuter$;
import org.apache.spark.sql.catalyst.plans.RightOuter$;
import org.apache.spark.sql.catalyst.plans.logical.BROADCAST$;
import org.apache.spark.sql.catalyst.plans.logical.CTERelationDef;
import org.apache.spark.sql.catalyst.plans.logical.CTERelationDef$;
import org.apache.spark.sql.catalyst.plans.logical.CTERelationRef;
import org.apache.spark.sql.catalyst.plans.logical.Filter;
import org.apache.spark.sql.catalyst.plans.logical.HintInfo;
import org.apache.spark.sql.catalyst.plans.logical.Join;
import org.apache.spark.sql.catalyst.plans.logical.JoinHint;
import org.apache.spark.sql.catalyst.plans.logical.LogicalPlan;
import org.apache.spark.sql.catalyst.plans.logical.Project;
import org.apache.spark.sql.catalyst.plans.logical.ResolvedHint;
import org.apache.spark.sql.catalyst.plans.logical.SubqueryAlias;
import org.apache.spark.sql.catalyst.plans.logical.View;
import org.apache.spark.sql.catalyst.plans.logical.WithCTE;
import org.apache.spark.sql.catalyst.rules.Rule;
import scala.Option;
import scala.PartialFunction;
import scala.collection.immutable.Seq;
import scala.jdk.javaapi.CollectionConverters;
import scala.runtime.BoxedUnit;

/** Lo que `spark.sql.extensions` carga: las reglas de ORE para Spark. */
public class ReglasDeOre implements scala.Function1<SparkSessionExtensions, BoxedUnit> {
  @Override
  public BoxedUnit apply(SparkSessionExtensions e) {
    e.injectPostHocResolutionRule(new scala.runtime.AbstractFunction1<SparkSession, Rule<LogicalPlan>>() {
      @Override
      public Rule<LogicalPlan> apply(SparkSession s) {
        return new JoinExternoEspacial();
      }
    });
    e.injectPostHocResolutionRule(new scala.runtime.AbstractFunction1<SparkSession, Rule<LogicalPlan>>() {
      @Override
      public Rule<LogicalPlan> apply(SparkSession s) {
        return new DifundirLaSubconsulta();
      }
    });
    e.injectOptimizerRule(new scala.runtime.AbstractFunction1<SparkSession, Rule<LogicalPlan>>() {
      @Override
      public Rule<LogicalPlan> apply(SparkSession s) {
        return new LadoGrandeALaDerecha();
      }
    });
    return BoxedUnit.UNIT;
  }

  // ── C3·c · La rejilla de Sedona, sobre el lado grande ────────────────────────────────────────────
  //
  // Medido en victor (C3·c0, 100 M puntos con el 90 % en 200 m² × 300 k zonas, `RangeJoin`): con la
  // rejilla hecha sobre las zonas (uniformes), los puntos densos caen en UNA celda y el join se queda en
  // un core (> 9 min sin acabar); con la rejilla sobre los puntos, los 7 cores, 687 s y sin OOM. AQE no
  // lo ve: Sedona reparte por su cuenta. Sedona hace la rejilla con el argumento del predicado que dice
  // `spark.sedona.join.spatitionside` (`right` en `spark-defaults.conf`) y el índice con el otro: esta regla
  // pone el lado con más filas (o más bytes, si Spark no sabe las filas) a la DERECHA del predicado.
  // `ST_Contains(zona, punto)`, la forma de siempre, no se toca.

  /** Los predicados que se pueden girar: el que da los mismos pares con los argumentos al revés. */
  static final Map<String, String> GIRO = Map.of(
      "ST_Contains", "ST_Within", "ST_Within", "ST_Contains", "ST_Covers", "ST_CoveredBy",
      "ST_CoveredBy", "ST_Covers", "ST_Intersects", "ST_Intersects", "ST_DWithin", "ST_DWithin");

  public static class LadoGrandeALaDerecha extends Rule<LogicalPlan> {
    @Override
    public LogicalPlan apply(LogicalPlan plan) {
      return plan.transformUp(PartialFunction.fromFunction(LadoGrandeALaDerecha::girar));
    }

    static LogicalPlan girar(LogicalPlan nodo) {
      if (!(nodo instanceof Join j) || j.joinType() != Inner$.MODULE$ || j.condition().isEmpty()) {
        return nodo;
      }
      // Lo difundido no se reparte: da igual el orden.
      if (JoinExternoEspacial.difunde(j.hint().leftHint()) || JoinExternoEspacial.difunde(j.hint().rightHint())
          || JoinExternoEspacial.marcado(j.left()) || JoinExternoEspacial.marcado(j.right())) {
        return nodo;
      }
      int cual = mayor(j.left(), j.right());   // > 0: el izquierdo del join es el grande
      if (cual == 0) {
        return nodo;
      }
      AttributeSet izq = j.left().outputSet();
      AttributeSet der = j.right().outputSet();
      Expression c = j.condition().get();
      Expression nueva = c.transform(PartialFunction.fromFunction((Expression x) -> {
        String n = x.getClass().getSimpleName();
        if (!GIRO.containsKey(n) || x.children().size() < 2) {
          return x;
        }
        Expression a = x.children().apply(0);
        Expression b = x.children().apply(1);
        // ¿Está el grande a la izquierda del predicado?
        boolean grandeDelante = cual > 0
            ? a.references().subsetOf(izq) && b.references().subsetOf(der)
            : a.references().subsetOf(der) && b.references().subsetOf(izq);
        if (!grandeDelante) {
          return x;
        }
        List<Expression> hijos = new ArrayList<>(lista(x.children()));
        hijos.set(0, b);
        hijos.set(1, a);
        try {
          return (Expression) Class.forName(x.getClass().getPackageName() + "." + GIRO.get(n))
              .getConstructor(Seq.class).newInstance(seq(hijos));
        } catch (ReflectiveOperationException e) {
          return x;
        }
      }));
      if (nueva.fastEquals(c)) {
        return nodo;
      }
      return new Join(j.left(), j.right(), j.joinType(), Option.apply(nueva), j.hint());
    }

    /** > 0 si `a` es más grande que `b`, < 0 si al revés, 0 si no se sabe o son iguales: por filas si
     *  Spark las sabe de los dos, si no por bytes. */
    static int mayor(LogicalPlan a, LogicalPlan b) {
      var sa = a.stats();
      var sb = b.stats();
      if (sa.rowCount().isDefined() && sb.rowCount().isDefined()) {
        return Integer.signum(sa.rowCount().get().compare(sb.rowCount().get()));
      }
      return Integer.signum(sa.sizeInBytes().compare(sb.sizeInBytes()));
    }
  }

  /** Los predicados de Sedona que hacen espacial un join (sus clases se llaman como la función). */
  static final Set<String> PREDICADOS = Set.of(
      "ST_Contains", "ST_Intersects", "ST_Within", "ST_Covers", "ST_CoveredBy", "ST_Crosses", "ST_Overlaps",
      "ST_Touches", "ST_Equals", "ST_DWithin", "ST_Distance", "ST_DistanceSphere", "ST_DistanceSpheroid");

  /** La regla: un join externo espacial sin el lado a difundir marcado → la forma de Sedona. */
  public static class JoinExternoEspacial extends Rule<LogicalPlan> {
    @Override
    public LogicalPlan apply(LogicalPlan plan) {
      return plan.transformUp(PartialFunction.fromFunction(JoinExternoEspacial::reescribir));
    }

    static LogicalPlan reescribir(LogicalPlan nodo) {
      if (!(nodo instanceof Join j) || j.condition().isEmpty()) {
        return nodo;
      }
      JoinType tipo = j.joinType();
      boolean izquierda = tipo == LeftOuter$.MODULE$;
      if (!izquierda && tipo != RightOuter$.MODULE$) {
        return nodo;
      }
      Expression p = j.condition().get();
      LogicalPlan conservado = izquierda ? j.left() : j.right();
      LogicalPlan otro = izquierda ? j.right() : j.left();
      if (!espacial(p, conservado.outputSet(), otro.outputSet())) {
        return nodo;
      }
      // Sedona ya lo indexa si el otro lado va difundido (la marca, en el join o en el hijo).
      Option<HintInfo> marca = izquierda ? j.hint().rightHint() : j.hint().leftHint();
      if (difunde(marca) || marcado(otro)) {
        return nodo;
      }

      // c = conservado + k, una vez (CTE); c1 la referencia que sale, c2 la del join interno.
      Alias numero = new Alias(new MonotonicallyIncreasingID(), "__ore_k", NamedExpression$.MODULE$.newExprId(),
          vacia(), Option.empty(), vacia());
      List<NamedExpression> conK = new ArrayList<>(lista(conservado.output()));
      conK.add(numero);
      CTERelationDef def = new CTERelationDef(new Project(seq(conK), conservado), CTERelationDef$.MODULE$.newId(),
          Option.empty(), false, Option.empty());
      CTERelationRef c1 = new CTERelationRef(def.id(), true, def.output(), false, Option.empty(), false,
          Option.empty(), false);
      CTERelationRef c2 = (CTERelationRef) c1.newInstance();
      List<Attribute> s1 = lista(c1.output());
      List<Attribute> s2 = lista(c2.output());
      Attribute k1 = s1.get(s1.size() - 1);
      Attribute k2 = s2.get(s2.size() - 1);
      Map<Object, Attribute> aC2 = new HashMap<>();
      for (int i = 0; i < s1.size(); i++) {
        aC2.put(s1.get(i).exprId(), s2.get(i));
      }
      Expression p2 = p.transform(PartialFunction.fromFunction(
          (Expression x) -> x instanceof Attribute a && aC2.containsKey(a.exprId()) ? aC2.get(a.exprId()) : x));

      // d = (k de c2, columnas del otro lado), del join INTERNO espacial (que Sedona indexa).
      Join interno = izquierda
          ? new Join(c2, otro, Inner$.MODULE$, Option.apply(p2), JoinHint.NONE())
          : new Join(otro, c2, Inner$.MODULE$, Option.apply(p2), JoinHint.NONE());
      List<NamedExpression> colsD = new ArrayList<>();
      colsD.add(k2);
      colsD.addAll(lista(otro.output()));
      Project d = new Project(seq(colsD), interno);

      // c1 externo por k con d, y las columnas de antes, en su orden y con sus ids.
      Join externo = izquierda
          ? new Join(c1, d, LeftOuter$.MODULE$, Option.apply(new EqualTo(k1, k2)), JoinHint.NONE())
          : new Join(d, c1, RightOuter$.MODULE$, Option.apply(new EqualTo(k2, k1)), JoinHint.NONE());
      List<NamedExpression> salida = new ArrayList<>(lista(j.output()));
      return new WithCTE(new Project(seq(salida), externo), seq(List.of(def)));
    }

    /** ¿`p` tiene un predicado de Sedona que mira a los dos lados? */
    static boolean espacial(Expression p, AttributeSet a, AttributeSet b) {
      return p.find(new scala.runtime.AbstractFunction1<Expression, Object>() {
        @Override
        public Object apply(Expression x) {
          return PREDICADOS.contains(x.getClass().getSimpleName())
              && x.references().intersect(a).nonEmpty() && x.references().intersect(b).nonEmpty();
        }
      }).isDefined();
    }

    /** ¿Lleva `l` la marca de difundir, bajo los envoltorios de una vista o un alias (`F.broadcast` de
     *  un DataFrame que luego es una vista, como hace `over()`)? */
    static boolean marcado(LogicalPlan l) {
      while (l instanceof SubqueryAlias || l instanceof View) {
        l = l.children().head();
      }
      return l instanceof ResolvedHint h && difunde(Option.apply(h.hints()));
    }

    static boolean difunde(Option<HintInfo> h) {
      return h.isDefined() && h.get().strategy().isDefined() && h.get().strategy().get() == BROADCAST$.MODULE$;
    }
  }

  // ── C3·e · La geometría de una subconsulta escalar, difundida ──────────────────────────────────────
  //
  // Q2 de SpatialBench: `WHERE ST_Intersects(t.punto, (SELECT z.geom FROM zone z WHERE … LIMIT 1))`. Spark
  // calcula la subconsulta una vez, pero Sedona no la ve como el lado de un join: en cada una de las 600 M
  // filas deserializa el polígono y lo cruza sin índice (> 600 s en C3·d). Con el polígono como lado
  // difundido de un join (q2d de C0, a mano) es un `BroadcastIndexJoin`: 125 s en un nodo.
  //
  //   Filter(… AND P(a, (subconsulta)) AND …, hijo)
  //     →   Project(hijo.*, Filter(…, Join(hijo, BROADCAST(subconsulta), Inner, P(a, g))))
  //
  // Solo si el predicado es un conjunto del AND (no dentro de un OR ni de un NOT: ahí el NULL de una
  // subconsulta vacía cambia el sentido), el otro argumento usa columnas del hijo, y la subconsulta no está
  // correlacionada, da una columna y como mucho UNA fila (`maxRows`, el `LIMIT 1` de Q2). Entonces da lo
  // mismo: con 0 filas el predicado es NULL y el Filter lo descarta todo, como un join interno contra nada;
  // con 1, el mismo cruce. En el analizador (post-hoc): la marca de difundir la recoge el optimizador.

  public static class DifundirLaSubconsulta extends Rule<LogicalPlan> {
    @Override
    public LogicalPlan apply(LogicalPlan plan) {
      return plan.transformUp(PartialFunction.fromFunction(DifundirLaSubconsulta::reescribir));
    }

    static LogicalPlan reescribir(LogicalPlan nodo) {
      if (!(nodo instanceof Filter f)) {
        return nodo;
      }
      LogicalPlan hijo = f.child();
      AttributeSet suyas = hijo.outputSet();
      List<Expression> partes = new ArrayList<>();
      conjuntos(f.condition(), partes);
      for (int i = 0; i < partes.size(); i++) {
        Expression x = partes.get(i);
        if (!GIRO.containsKey(x.getClass().getSimpleName()) || x.children().size() < 2) {
          continue;
        }
        for (int k = 0; k < 2; k++) {
          Expression a = x.children().apply(k);
          Expression b = x.children().apply(1 - k);
          List<ScalarSubquery> qs = new ArrayList<>();
          subconsultas(b, qs);
          if (qs.size() != 1 || a.references().isEmpty() || !a.references().subsetOf(suyas)
              || !b.references().isEmpty()) {
            continue;
          }
          ScalarSubquery q = qs.get(0);
          LogicalPlan sub = q.plan();
          Option<Object> filas = sub.maxRows();
          if (!q.outerAttrs().isEmpty() || sub.output().size() != 1 || filas.isEmpty()
              || ((Long) filas.get()) > 1) {
            continue;
          }
          Attribute g = sub.output().head();
          Expression p = x.transform(PartialFunction.fromFunction(
              (Expression e) -> e instanceof ScalarSubquery s2 && s2.exprId().equals(q.exprId()) ? g : e));
          Join join = new Join(hijo, new ResolvedHint(sub, new HintInfo(Option.apply(BROADCAST$.MODULE$))),
              Inner$.MODULE$, Option.apply(p), JoinHint.NONE());
          Expression resto = null;
          for (int r = 0; r < partes.size(); r++) {
            if (r != i) {
              resto = resto == null ? partes.get(r) : new And(resto, partes.get(r));
            }
          }
          LogicalPlan arriba = resto == null ? join : new Filter(resto, join);
          return new Project(seq(new ArrayList<NamedExpression>(lista(hijo.output()))), arriba);
        }
      }
      return nodo;
    }

    /** Los conjuntos de un AND, en orden. */
    static void conjuntos(Expression e, List<Expression> out) {
      if (e instanceof And y) {
        conjuntos(y.left(), out);
        conjuntos(y.right(), out);
      } else {
        out.add(e);
      }
    }

    static void subconsultas(Expression e, List<ScalarSubquery> out) {
      if (e instanceof ScalarSubquery q) {
        out.add(q);
        return;
      }
      for (Expression c : ReglasDeOre.<Expression>lista(e.children())) {
        subconsultas(c, out);
      }
    }
  }

  @SuppressWarnings("unchecked")
  static <T> List<T> lista(scala.collection.Seq<? extends T> s) {
    return new ArrayList<>((java.util.Collection<T>) CollectionConverters.asJava(s));
  }

  static <T> Seq<T> seq(List<T> l) {
    return CollectionConverters.asScala(l).toList();
  }

  static <T> Seq<T> vacia() {
    return seq(List.of());
  }
}
