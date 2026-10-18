// ORE 0068 C2·3 · LA IDENTIDAD DEL PUESTO, EN CADA PETICIÓN DE SPARK AL CATÁLOGO.
//
// El pod se presenta con su token de Kubernetes (`x-ore-pod`, 51-el-puesto.yaml): dura 3600 s y el
// kubelet lo reescribe en el sitio. `header.*` de Iceberg se lee UNA vez, al abrir el catálogo, así
// que un Build de más de una hora hablaría con un token caducado. Esto lo lee del fichero en CADA
// petición, por el SPI `AuthManager` de Iceberg 1.12 (`rest.auth.type` = esta clase). Medido en el
// laboratorio de la pieza 0 (`pruebas-de-fuego/geo/c2/trabajo-spark/`): cambiando el fichero a mitad
// de sesión, va · 403 · va.
//
//   ore.identidad.fichero   de dónde leerla (el token del pod)
//   ore.identidad.cabecera  en qué cabecera (x-ore-pod)
//
// La compila la etapa `puesto-spark` del Dockerfile contra los jars de Spark, y va con ellos.
package ore.spark;

import java.io.IOException;
import java.io.UncheckedIOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import org.apache.iceberg.rest.HTTPHeaders;
import org.apache.iceberg.rest.HTTPRequest;
import org.apache.iceberg.rest.ImmutableHTTPRequest;
import org.apache.iceberg.rest.RESTClient;
import org.apache.iceberg.rest.auth.AuthManager;
import org.apache.iceberg.rest.auth.AuthSession;

public class IdentidadDelPuesto implements AuthManager {
  public IdentidadDelPuesto(String nombre) {}

  @Override
  public AuthSession catalogSession(RESTClient cliente, Map<String, String> p) {
    String f = p.get("ore.identidad.fichero");
    if (f == null || f.isBlank()) {
      throw new IllegalArgumentException("ore.identidad.fichero: de dónde se lee la identidad del puesto");
    }
    Path fichero = Path.of(f);
    String cabecera = p.getOrDefault("ore.identidad.cabecera", "x-ore-pod");
    return new AuthSession() {
      @Override
      public HTTPRequest authenticate(HTTPRequest r) {
        String valor;
        try {
          valor = Files.readString(fichero).strip();
        } catch (IOException e) {
          throw new UncheckedIOException("la identidad del puesto no se lee: " + fichero, e);
        }
        // La del fichero manda sobre una estática con el mismo nombre.
        HTTPHeaders h = HTTPHeaders.of(Map.of(cabecera, valor)).putIfAbsent(r.headers());
        return ImmutableHTTPRequest.builder().from(r).headers(h).build();
      }

      @Override
      public void close() {}
    };
  }

  @Override
  public void close() {}
}
