FROM python:3.12-slim
RUN pip install --no-cache-dir -q "psycopg[binary]==3.2.3"
COPY conciliador.py /app/
CMD ["python", "-u", "/app/conciliador.py"]
