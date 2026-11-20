# Resumen simple del trial de skills de just-right-rust (al 2026-11-20)

Detalle: `TRIAL-LOG.md` (v1), `v2/REPORT.md` (v2), `v2/STATE.md` (bitacora con todas las tablas).
Todo es local; nada pusheado ni publicado.

## La pregunta

En que circunstancias los skills nuevos de just-right-rust hacen que un agente escriba mejor Rust.

## Que se hizo

- v1: 16 refactors agenticos del crate SQS de aws-emulator (19.6k lineas), revision ciega por run.
- v2: 87 runs sobre un servicio chico ("miniq") con tests de aceptacion ocultos, variando
  tarea (construir de cero, agregar feature, refactor), forma de entrega y modelo
  (Haiku, Sonnet, Opus). Revision ciega por dimension, cada revisor ve todas las runs juntas.
- Todas las runs de v2 pasan los tests ocultos: ninguna mejora costo correccion.

## Formas de entrega probadas

| Nombre | Que recibe el agente |
|---|---|
| none | nada |
| skills | los 8 skills como estan (guias de ~200 lineas cada una) |
| router | skills + un skill de entrada que manda a cargar los demas |
| audit | skills + un skill corto con "que buscar" en codigo existente |
| context | sin plugin; los checklists de los skills (68 lineas) en CLAUDE.md |
| hook | skills + un hook que corre un detector y devuelve hallazgos antes de terminar |
| short | un solo skill con esas mismas 68 lineas |
| limits / context2 | checklist + los limites "no sobre-aplicar" (como skill / en CLAUDE.md) |

## Resultado por circunstancia

| Tarea | Los skills se cargan solos? | Que funciono | Que no |
|---|---|---|---|
| Construir de cero | Si en el brazo skills (Sonnet 3 de 3, Opus 2 de 2, Haiku 1 de 2); con el hook presente solo 1 de 3 al inicio | Todo: puntaje ciego 6 -> 11 a 13 de 16. Mejor: un skill corto de 68 lineas | - |
| Refactor | Solo `rust-keep-it-simple` (6 de 6); nunca tipos, errores, ownership ni tests | Checklist en CLAUDE.md, skill de auditoria, hook con detector | Cargar las guias completas: identico a no hacer nada |
| Agregar feature | No (0 de 3) | Nada se separo de "none" | Todo: el codigo nuevo copia el estilo que encuentra |
| Refactor del crate grande (SQS) | No | Skill de auditoria: 7 y 9 puntos en 10-12 min (v1 necesito 100+ turnos para eso). Es el resultado menos independiente: escribi ese skill conociendo la auditoria de SQS | Guias cargadas con enrutador |

## Las cinco conclusiones

1. Los skills si aportan valor cuando se escribe codigo nuevo. Sin solapamiento entre runs con
   y sin skills, en los tres modelos. Haiku con el checklist le gana a Opus sin nada.
2. Lo que actua es una lista corta, no la guia larga. 68 lineas rinden igual o mas que 60 KB de
   guias, y da igual si van en CLAUDE.md o en un skill.
3. Sobre codigo existente el agente necesita que le digan que buscar. Cargar guias de "como
   escribir" no cambia nada: en refactor, las 7 guias cargadas dan exactamente el crate original.
4. La lista corta sobre-aplica tipos (ultima en simplicidad). Los limites "no sobre-aplicar" lo
   corrigen a costa de menos tipos: es un intercambio, la suma queda casi igual. Que las guias
   completas den mejor "overall" salio en una revision y no se repitio en la segunda: abierto.
5. Al agregar una feature a un codigo con otro estilo, ninguna variante probada tuvo efecto.

Causa concreta encontrada en el texto: `rust-keep-it-simple` termina con "Apply these rules to
the code the change touches. Do not rewrite working code elsewhere". En un refactor eso le dice
al agente que no haga nada.

## Que recomendaria para just-right-rust

- Un skill de entrada corto (el checklist) como puerta; las guias quedan como referencia.
- Un skill de auditoria para pedidos de limpieza o review.
- Corregir la frase de alcance de `rust-keep-it-simple`.
- Usar el motor de hooks como detector que devuelve hallazgos.
- Agregar feature queda sin resolver.

## Limites

Un solo tipo de servicio, crates chicos en v2, 2 o 3 runs por celda, revisores y autores de la
misma familia de modelos, sin revision humana, y los textos de los brazos los escribi yo. Los
puntajes ciegos solo son comparables dentro de una misma fase (la escala de los revisores se
movio entre fases). Fiabilidad: de 85 puntajes repetidos por otro revisor, 82 difieren en 1
punto o menos y 46 son identicos.
