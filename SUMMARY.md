# Resumen final del trial de skills de just-right-rust (al 2026-11-23)

Detalle completo: `v2/REPORT.md` (secciones 1 a 10: segunda etapa; seccion 11: tercera etapa),
`TRIAL-LOG.md` (primera etapa), `v2/STATE.md` (bitacora). Repo: `fnzko/jrr-skills-trial` (privado).

## La pregunta

En que circunstancias los skills de just-right-rust hacen que un agente escriba mejor Rust, y como
hay que entregarlos para que eso pase.

## Que se hizo

| Etapa | Que se corrio |
|---|---|
| 1 | 16 refactors agenticos del crate SQS de aws-emulator, con y sin skills |
| 2 | 87 runs sobre un servicio chico con tests ocultos: tres tareas, ocho formas de entrega, tres modelos |
| 3 | 54 runs con hooks, 72 sondas de descripciones, y consumo de tokens y tiempo de todo |

Toda la calidad se midio con revision a ciegas. De 141 runs sobre el servicio chico, 140 pasan todos
los tests ocultos: ninguna mejora costo correccion.

## Lo que se encontro

1. **Las reglas sirven.** Con el checklist el codigo mejora de 6 a 13 sobre 16 al escribir de cero,
   en Haiku, Sonnet y Opus. Haiku con el checklist le gana a Opus sin nada.
2. **El problema es que no se cargan.** El agente elige skills una sola vez, al inicio, mirando el
   pedido. Las seis descripciones actuales nombran micro-actividades, no pedidos. Reescribirlas no
   alcanza: 1 carga en 45 sondas, incluso siguiendo la guia publicada.
3. **Lo que si carga siempre:** un skill de entrada, o tres skills por tarea (de cero, refactor,
   agregar feature). 27 de 27 sondas, y siempre el skill correcto.
4. **Un hook lo resuelve sin depender del agente.** Checklist al inicio de la sesion mas un detector
   al terminar rinde igual que tener el checklist en `CLAUDE.md`.
5. **Corto le gana a largo.** 68 lineas de checklist rinden igual o mas que las guias completas.
6. **Sobre codigo existente hay que decir que buscar.** Las guias cargadas dan el crate original.
7. **Agregar una feature sigue flojo.** El codigo nuevo copia el estilo que encuentra. En la ultima
   medicion las reglas suman unos 2 puntos, pero no es concluyente.

## Resultado final por forma de entrega

Puntaje a ciegas y costo respecto de no usar nada (Sonnet, 3 runs por celda).

| Entrega | De cero (max 16) | Refactor (max 12) | Feature (max 12) | Costo de cero |
|---|---|---|---|---|
| Hook inicio + detector | 13,0 | 9,0 | 9,0 | +68% |
| Checklist en `CLAUDE.md` | 12,7 | 8,7 | 9,7 | +69% |
| Checklist corto como skill | 12,7 | 7,3 | 9,3 | +27% |
| Hook solo al inicio | 11,3 | 7,3 | 9,3 | +59% |
| Sin nada | 6,3 | 5,0 | 7,0 | 0 |

El costo extra es trabajo hecho (mas tipos, mas tests), no texto de reglas en contexto.

## Que recomendaria para just-right-rust

1. Un skill de entrada corto con el checklist (o tres por tarea). Las seis guias quedan como
   referencia citada desde ahi.
2. Hooks en el plugin: checklist al inicio y detector al terminar. Donde no hay hooks, queda el skill.
3. Corregir la frase de `rust-keep-it-simple` que frena los refactors ("Do not rewrite working code
   elsewhere").
4. Para el harness de evals de James: sumar una metrica de calidad, porque hoy mide solo si pasan
   los tests y estos skills cambian como queda el codigo, no si funciona.

## Sin resolver

- Agregar una feature a codigo con otro estilo.
- La frase de alcance corregida y los tres skills con contenido distinto no se midieron con runs
  completas.

## Limites

Dos o tres runs por celda, un solo tipo de servicio, crates chicos, revisores y autores de la misma
familia de modelos, sin revision humana, y los textos de las variantes los escribi yo. Los puntajes
solo se comparan dentro de una misma tanda. Fiabilidad de revisores: de 123 puntajes repetidos, 122
difieren en 1 punto o menos.
