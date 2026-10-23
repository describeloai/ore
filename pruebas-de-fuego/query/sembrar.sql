-- Query · la escalera: 3, 30 y 300 M viajes sintéticos (range()), para medir Query (Q4·3).
-- Una sentencia por dataset: el tamaño lo pone la admisión (3 M en el puesto de la hoja, 30 y 300 M en G).
-- Requiere la base `query`: si no existe, antes `create standard database query;`.

create or replace dataset query.default.trip_3m as
select
  range as t_tripkey,
  cast(hash(range, 1) % 1500000 as bigint) as t_custkey,
  cast(hash(range, 2) % 50000 as bigint) as t_driverkey,
  cast(hash(range, 3) % 100000 as bigint) as t_vehiclekey,
  timestamp '2025-01-01' + to_seconds(cast(hash(range, 4) % 31536000 as bigint)) as t_pickuptime,
  round(cast(hash(range, 5) % 30000 as double) / 1000, 3) as t_distance,
  round(2.5 + cast(hash(range, 5) % 30000 as double) / 1000 * 1.8, 2) as t_fare,
  round(cast(hash(range, 6) % 1500 as double) / 100, 2) as t_tip,
  -74.25 + cast(hash(range, 7) % 500000 as double) / 1000000 as t_pickup_lon,
  40.50 + cast(hash(range, 8) % 400000 as double) / 1000000 as t_pickup_lat,
  -74.25 + cast(hash(range, 9) % 500000 as double) / 1000000 as t_dropoff_lon,
  40.50 + cast(hash(range, 10) % 400000 as double) / 1000000 as t_dropoff_lat
from range(3000000);

create or replace dataset query.default.trip_30m as
select
  range as t_tripkey,
  cast(hash(range, 1) % 1500000 as bigint) as t_custkey,
  cast(hash(range, 2) % 50000 as bigint) as t_driverkey,
  cast(hash(range, 3) % 100000 as bigint) as t_vehiclekey,
  timestamp '2025-01-01' + to_seconds(cast(hash(range, 4) % 31536000 as bigint)) as t_pickuptime,
  round(cast(hash(range, 5) % 30000 as double) / 1000, 3) as t_distance,
  round(2.5 + cast(hash(range, 5) % 30000 as double) / 1000 * 1.8, 2) as t_fare,
  round(cast(hash(range, 6) % 1500 as double) / 100, 2) as t_tip,
  -74.25 + cast(hash(range, 7) % 500000 as double) / 1000000 as t_pickup_lon,
  40.50 + cast(hash(range, 8) % 400000 as double) / 1000000 as t_pickup_lat,
  -74.25 + cast(hash(range, 9) % 500000 as double) / 1000000 as t_dropoff_lon,
  40.50 + cast(hash(range, 10) % 400000 as double) / 1000000 as t_dropoff_lat
from range(30000000);

create or replace dataset query.default.trip_300m as
select
  range as t_tripkey,
  cast(hash(range, 1) % 1500000 as bigint) as t_custkey,
  cast(hash(range, 2) % 50000 as bigint) as t_driverkey,
  cast(hash(range, 3) % 100000 as bigint) as t_vehiclekey,
  timestamp '2025-01-01' + to_seconds(cast(hash(range, 4) % 31536000 as bigint)) as t_pickuptime,
  round(cast(hash(range, 5) % 30000 as double) / 1000, 3) as t_distance,
  round(2.5 + cast(hash(range, 5) % 30000 as double) / 1000 * 1.8, 2) as t_fare,
  round(cast(hash(range, 6) % 1500 as double) / 100, 2) as t_tip,
  -74.25 + cast(hash(range, 7) % 500000 as double) / 1000000 as t_pickup_lon,
  40.50 + cast(hash(range, 8) % 400000 as double) / 1000000 as t_pickup_lat,
  -74.25 + cast(hash(range, 9) % 500000 as double) / 1000000 as t_dropoff_lon,
  40.50 + cast(hash(range, 10) % 400000 as double) / 1000000 as t_dropoff_lat
from range(300000000);
