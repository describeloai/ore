select g, count(*) n,
       avg(t_dropofftime - t_pickuptime) avg_intervalo,
       avg(epoch_ms(t_dropofftime) - epoch_ms(t_pickuptime)) avg_ms_exacto,
       min(epoch_ms(t_dropofftime) - epoch_ms(t_pickuptime)) min_ms,
       max(epoch_ms(t_dropofftime) - epoch_ms(t_pickuptime)) max_ms,
       count(*) filter (where (t_dropofftime - t_pickuptime) >= interval 1 day) de_un_dia_o_mas
from (select *, 'todo' g from trip union all select *, '484552' g from (select * from trip limit 484552)
      union all select *, '19' g from (select * from trip limit 19))
group by g order by n
