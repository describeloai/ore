"""ORE 0072 F2·3·2 · un lector de PMTiles v3 que cuenta lo que pide (sin dependencias: va igual en el puesto).

`Lector(leer)` con `leer(desde, largo) -> bytes` (un GET por rango). La cabecera y el directorio raíz se piden
juntos (los primeros 16 KB, como hace el cliente de PMTiles); los directorios hoja, una vez cada uno (en caché).
`tesela(z, x, y)` devuelve los bytes (sin descomprimir) o None. `gets` cuenta las peticiones.
"""
import gzip
import struct
import threading


def _varint(b, i):
    r = s = 0
    while True:
        c = b[i]
        i += 1
        r |= (c & 0x7F) << s
        if c < 0x80:
            return r, i
        s += 7


def directorio(b):
    n, i = _varint(b, 0)
    ids, corridas, largos, desplaz = [], [], [], []
    ultimo = 0
    for _ in range(n):
        d, i = _varint(b, i)
        ultimo += d
        ids.append(ultimo)
    for _ in range(n):
        v, i = _varint(b, i)
        corridas.append(v)
    for _ in range(n):
        v, i = _varint(b, i)
        largos.append(v)
    for k in range(n):
        v, i = _varint(b, i)
        desplaz.append(desplaz[k - 1] + largos[k - 1] if v == 0 and k > 0 else v - 1)
    return list(zip(ids, corridas, largos, desplaz))


def tile_id(z, x, y):
    """El id de PMTiles: las teselas de los zooms anteriores + la posición en la curva de Hilbert del zoom."""
    acc = ((1 << (2 * z)) - 1) // 3
    n = 1 << z
    d = 0
    s = n >> 1
    while s > 0:
        rx = 1 if x & s else 0
        ry = 1 if y & s else 0
        d += s * s * ((3 * rx) ^ ry)
        if ry == 0:
            if rx == 1:
                x = s - 1 - x
                y = s - 1 - y
            x, y = y, x
        s >>= 1
    return acc + d


def _buscar(entradas, tid):
    a, b = 0, len(entradas) - 1
    while a <= b:
        m = (a + b) // 2
        if entradas[m][0] < tid:
            a = m + 1
        elif entradas[m][0] > tid:
            b = m - 1
        else:
            return entradas[m]
    if b >= 0:
        e = entradas[b]
        if e[1] == 0 or tid - e[0] < e[1]:
            return e
    return None


class Lector:
    def __init__(self, leer):
        self._leer = leer
        self.gets = 0
        self._candado = threading.Lock()
        self._abriendo = threading.Lock()
        self._hojas = {}
        self.cabecera = None

    def _get(self, desde, largo):
        with self._candado:
            self.gets += 1
        return self._leer(desde, largo)

    def _abrir(self):
        if self.cabecera is not None:
            return
        # las teselas de una vista llegan a la vez: la cabecera, una sola vez (las demás esperan)
        with self._abriendo:
            if self.cabecera is None:
                self._abrir_ya()

    def _abrir_ya(self):
        b = self._get(0, 16384)
        if b[:7] != b"PMTiles" or b[7] != 3:
            raise ValueError("no es PMTiles v3")
        (raiz_o, raiz_l, _mo, _ml, hojas_o, _hl, datos_o, _dl, _a, _e, _c) = struct.unpack_from("<11Q", b, 8)
        comp_interna = b[97]
        self._des = (lambda x: gzip.decompress(x)) if comp_interna == 2 else (lambda x: x)
        raiz = b[raiz_o: raiz_o + raiz_l] if raiz_o + raiz_l <= len(b) else self._get(raiz_o, raiz_l)
        self._raiz = directorio(self._des(raiz))
        # la cabecera, la última: quien la ve puesta tiene ya la raíz
        self.cabecera = {"hojas": hojas_o, "datos": datos_o, "tesela_gzip": b[98] == 2,
                         "minzoom": b[100], "maxzoom": b[101]}

    def tesela(self, z, x, y):
        self._abrir()
        tid = tile_id(z, x, y)
        entradas = self._raiz
        for _ in range(4):
            e = _buscar(entradas, tid)
            if e is None:
                return None
            if e[1] > 0:
                return self._get(self.cabecera["datos"] + e[3], e[2])
            # un directorio hoja (una vez: luego en caché)
            clave = (e[3], e[2])
            if clave not in self._hojas:
                self._hojas[clave] = directorio(self._des(self._get(self.cabecera["hojas"] + e[3], e[2])))
            entradas = self._hojas[clave]
        return None
