DROP TABLE ore.tipos PURGE;
CREATE TABLE ore.tipos (
  id            NUMBER(10) PRIMARY KEY,
  n_sin_prec    NUMBER,
  n_entero      NUMBER(18,0),
  n_grande      NUMBER(38,0),
  n_dec         NUMBER(12,2),
  n_neg_escala  NUMBER(10,-2),
  f_float       FLOAT(53),
  b_float       BINARY_FLOAT,
  b_double      BINARY_DOUBLE,
  c_char        CHAR(5),
  c_varchar     VARCHAR2(50),
  c_nvarchar    NVARCHAR2(50),
  d_date        DATE,
  t_ts          TIMESTAMP(6),
  t_ts9         TIMESTAMP(9),
  t_tstz        TIMESTAMP(6) WITH TIME ZONE,
  t_tsltz       TIMESTAMP(6) WITH LOCAL TIME ZONE,
  i_ym          INTERVAL YEAR(4) TO MONTH,
  i_ds          INTERVAL DAY(5) TO SECOND(6),
  r_raw         RAW(16),
  l_clob        CLOB,
  l_blob        BLOB,
  x_xml         XMLTYPE,
  j_json        JSON,
  b_bool        BOOLEAN
);
INSERT INTO ore.tipos VALUES (1, 3.14159265358979323846264338327950288, 123456789012345678, 99999999999999999999999999999999999999,
  1234567890.12, 12300, 1.5, 1.5, 2.718281828459045, 'ab', 'texto ñ €', N'nvarchar ñ',
  DATE '2026-10-25' + 13/24, TIMESTAMP '2026-10-25 13:14:15.123456', TIMESTAMP '2026-10-25 13:14:15.123456789',
  TIMESTAMP '2026-10-25 13:14:15.123456 +05:30', TIMESTAMP '2026-10-25 13:14:15.123456 +00:00',
  INTERVAL '12-3' YEAR TO MONTH, INTERVAL '5 04:03:02.000001' DAY TO SECOND,
  HEXTORAW('00FF10AB'), TO_CLOB('clob largo'), UTL_RAW.CAST_TO_RAW('blob'),
  XMLTYPE('<a><b>1</b></a>'), JSON('{"k":[1,2,{"z":null}]}'), TRUE);
INSERT INTO ore.tipos (id) VALUES (2);
INSERT INTO ore.tipos (id, n_sin_prec, n_entero, d_date, t_tstz, c_varchar, f_float, b_double) VALUES (3, -0.000000000000000000000000000000000000001, -999999999999999999, DATE '0001-01-01', TIMESTAMP '1969-12-31 23:59:59.999999 -08:00', '', 1E100, BINARY_DOUBLE_INFINITY);
COMMIT;
EXIT
