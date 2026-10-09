# El «cómputo» del laboratorio de P5 y P6: lo que de la VM de Neon le importa al proxy y al plano de control.
#   Postgres 17 en 5432, pgbouncer 1.24.1 en 6432 con el pgbouncer.ini de la imagen de cómputo de Neon
#   (compute/etc/pgbouncer.ini en el commit fijado) y, como proceso principal, el compute_ctl de mentira
#   (compute_ctl_falso.py): /status, /configure, /terminate en 3080, con el contrato del real (P6·0).
FROM postgres:17
RUN apt-get update -qq && apt-get install -y -qq --no-install-recommends pgbouncer=1.24.1-1+deb13u2 python3 \
 && rm -rf /var/lib/apt/lists/*
COPY pgbouncer.ini /etc/pgbouncer.ini
COPY compute_ctl_falso.py /usr/local/bin/compute_ctl_falso.py
EXPOSE 3080 5432 6432
ENTRYPOINT []
CMD ["python3", "-u", "/usr/local/bin/compute_ctl_falso.py"]
