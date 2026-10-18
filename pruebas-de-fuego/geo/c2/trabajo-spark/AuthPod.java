// ORE 0068 C2·3 pieza 0 · ¿Puede Spark mandar a /v1 una identidad que CAMBIA mientras corre? En el
// clúster es el token proyectado del pod (51-el-puesto.yaml: 3600 s, el kubelet reescribe el fichero),
// y `header.x-ore-pod` de Iceberg se lee UNA vez. Esto lo lee del fichero en CADA petición, por el SPI
// `AuthManager` de Iceberg 1.12 (`rest.auth.type` = esta clase). Propiedades:
//   ore.identidad.fichero   de dónde leerla
//   ore.identidad.cabecera  en qué cabecera mandarla (en el clúster, x-ore-pod)
package ore;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import org.apache.iceberg.rest.HTTPHeaders;
import org.apache.iceberg.rest.HTTPRequest;
import org.apache.iceberg.rest.ImmutableHTTPRequest;
import org.apache.iceberg.rest.RESTClient;
import org.apache.iceberg.rest.auth.AuthManager;
import org.apache.iceberg.rest.auth.AuthSession;

public class AuthPod implements AuthManager {
  public AuthPod(String nombre) {}

  @Override
  public AuthSession catalogSession(RESTClient cliente, Map<String, String> p) {
    Path fichero = Path.of(p.get("ore.identidad.fichero"));
    String cabecera = p.getOrDefault("ore.identidad.cabecera", "x-ore-pod");
    return new AuthSession() {
      @Override
      public HTTPRequest authenticate(HTTPRequest r) {
        String valor;
        try {
          valor = Files.readString(fichero).strip();
        } catch (java.io.IOException e) {
          throw new java.io.UncheckedIOException("la identidad del puesto no se lee: " + fichero, e);
        }
        // La del fichero MANDA sobre una estática con el mismo nombre.
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
