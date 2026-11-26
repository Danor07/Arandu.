"""Umbral de mutación sobre uno o más outcomes.json de cargo-mutants (los
fragmentos de una misma corrida se juzgan juntos).

Exige 90 % de mutantes detectados y ningún timeout ni falla: un mutante que
cuelga las pruebas no se cuenta como detectado. Los inviables no compilan y no
miden nada, como en cargo-mutants. Cada fragmento trae su propia línea base,
que tiene que haber pasado.

Uso: python umbral_mutacion.py fragmento-1/outcomes.json [...]
"""
import json
import sys

UMBRAL = 90.0


def juzgar(reportes):
    conteo = {}
    for reporte in reportes:
        resultados = reporte.get("outcomes")
        if not isinstance(resultados, list):
            raise ValueError("no es un outcomes.json de cargo-mutants")
        base = [r for r in resultados if r.get("scenario") == "Baseline"]
        if len(base) != 1 or base[0].get("summary") != "Success":
            raise ValueError("la linea base de cargo-mutants no paso")
        for r in resultados:
            if r.get("scenario") != "Baseline":
                conteo[r.get("summary")] = conteo.get(r.get("summary"), 0) + 1
    detectados = conteo.get("CaughtMutant", 0)
    medidos = sum(n for s, n in conteo.items() if s != "Unviable")
    if medidos == 0:
        raise ValueError("ningun mutante medido")
    puntaje = 100.0 * detectados / medidos
    otros = {s: n for s, n in conteo.items() if s not in ("CaughtMutant", "MissedMutant", "Unviable")}
    return puntaje, conteo, otros


def main(rutas):
    reportes = [json.load(open(r, encoding="utf-8")) for r in rutas]
    puntaje, conteo, otros = juzgar(reportes)
    print(f"mutacion: {puntaje:.2f} % detectados; {conteo}")
    if otros:
        print(f"rechazado: resultados que no son deteccion ni sobrevivencia: {otros}")
        return 1
    if puntaje < UMBRAL:
        print(f"rechazado: por debajo del umbral de {UMBRAL} %")
        return 1
    return 0


def _autoprueba():
    base = {"scenario": "Baseline", "summary": "Success"}
    m = lambda s: {"scenario": {"Mutant": {}}, "summary": s}
    ok = {"outcomes": [base] + [m("CaughtMutant")] * 9 + [m("MissedMutant"), m("Unviable")]}
    assert juzgar([ok])[0] == 90.0
    bajo = {"outcomes": [base] + [m("CaughtMutant")] * 8 + [m("MissedMutant")] * 2}
    assert juzgar([bajo])[0] == 80.0
    assert juzgar([{"outcomes": [base, m("Timeout")]}])[2] == {"Timeout": 1}
    for malo in ({"outcomes": [m("CaughtMutant")]}, {"outcomes": [base, m("Unviable")]}, {}):
        try:
            juzgar([malo])
        except ValueError:
            continue
        raise AssertionError(malo)


if __name__ == "__main__":
    if sys.argv[1:] == ["--autoprueba"]:
        _autoprueba()
        print("autoprueba ok")
        sys.exit(0)
    sys.exit(main(sys.argv[1:]))
