import marimo

__generated_with = "0.25.1"
app = marimo.App()


@app.cell
def _():
    import marimo as mo
    import numpy as np
    import pandas as pd
    return mo, np, pd


@app.cell
def _(mo):
    n = mo.ui.slider(10, 200_000, value=1_000, label="filas", debounce=True)
    n
    return (n,)


@app.cell
def _(n, np, pd):
    df = pd.DataFrame({"k": np.arange(n.value) % 7, "v": np.random.default_rng(0).random(n.value)})
    df
    return (df,)


@app.cell
def _(df, mo):
    resumen = mo.sql(
        """
        select k, count(*) as n, avg(v) as media
        from df
        group by k
        order by k
        """
    )
    return (resumen,)


@app.cell
def _(mo, n):
    mo.md(f"**{n.value:,}** filas · el relevo de M1·a")
    return


if __name__ == "__main__":
    app.run()
