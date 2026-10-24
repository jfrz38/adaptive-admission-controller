# Adaptive Admission Controller

Núcleo portable de **control de admisión adaptativo**, priorización de solicitudes y ajuste dinámico de concurrencia, escrito en Rust y diseñado para distribuirse mediante WebAssembly.

> [!IMPORTANT]
> El proyecto se encuentra en fase de diseño. Este repositorio define inicialmente el alcance y la arquitectura antes de implementar el núcleo.

## Motivación

Los limitadores tradicionales suelen aceptar o rechazar tráfico a partir de una cuota fija. Este proyecto pretende adaptar la admisión al estado observado del servicio:

- Reducir carga cuando el servicio se aproxima a la saturación.
- Proteger primero las solicitudes de mayor prioridad.
- Regular el número de ejecuciones concurrentes.
- Ajustar el umbral de admisión mediante realimentación PID.
- Aprender de la latencia y del throughput observados.
- Compartir exactamente el mismo algoritmo entre lenguajes y gateways.

El proyecto parte de la experiencia obtenida con [`rate-limit-pid-controller`](https://github.com/jfrz38/rate-limit-pid-controller), pero no pretende hacer una traducción literal de su implementación TypeScript. La frontera del núcleo se rediseñará para que no dependa de Node.js, de un framework HTTP ni de un runtime asíncrono concreto.

## Objetivos

1. Implementar en Rust la lógica común de:
   - Prioridades y cohortes.
   - Admisión y rechazo.
   - Cola priorizada y expulsión por timeout.
   - Límites de concurrencia.
   - Estadísticas por ventanas de ejecución.
   - Control PID.
   - Ajuste automático de latencia y concurrencia.
2. Compilar el núcleo a WebAssembly para reutilizarlo desde distintos hosts.
3. Mantener el núcleo determinista, síncrono y sin dependencias de infraestructura.
4. Usar cero dependencias externas de producción en el crate del algoritmo siempre que no perjudique su seguridad o mantenibilidad.
5. Permitir adaptadores idiomáticos para Node.js, otros lenguajes y gateways.
6. Conservar una semántica comprobable mediante pruebas de conformidad compartidas.

## No objetivos

- Ejecutar dentro de WebAssembly callbacks o `Promise` pertenecientes al lenguaje anfitrión.
- Incluir un runtime asíncrono en el núcleo.
- Gestionar directamente threads, sockets, señales del proceso o peticiones HTTP.
- Proporcionar persistencia o coordinación distribuida desde el núcleo.
- Prometer que un único binario WebAssembly funcione sin adaptación en todos los gateways.
- Garantizar límites globales entre procesos, workers o nodos sin coordinación externa.

## Modelo de ejecución

El núcleo actuará como una **máquina de estados**. El host conservará las peticiones y ejecutará el trabajo; el núcleo decidirá qué debe ocurrir.

Una API conceptual podría tener esta forma:

```text
Controller::new(config)

submit(priority, now_ms)                  -> Actions
complete(request_id, outcome, now_ms)     -> Actions
advance_time(now_ms)                      -> Actions
shutdown(now_ms)                          -> Actions
next_deadline()                           -> Option<Timestamp>
```

Las acciones devueltas representarían decisiones como:

```text
Start(request_id)
Reject(request_id, reason)
Evict(request_id, reason)
Cancel(request_id)
ThresholdChanged(value)
ConcurrencyChanged(value)
```

El adaptador de cada plataforma será responsable de:

- Asociar el identificador con la petición, callback o continuación real.
- Ejecutar las acciones `Start`.
- Notificar al núcleo la finalización o el fallo.
- Programar la siguiente llamada temporal.
- Traducir errores y decisiones al modelo del framework o gateway.
- Integrar logging, métricas y ciclo de vida.

## Por qué el host controla el tiempo

WebAssembly no ofrece de forma universal temporizadores, reloj monotónico, event loop o threads. Para no depender de un runtime concreto:

- El host proporciona timestamps monotónicos.
- El núcleo calcula su siguiente deadline.
- El host programa el temporizador.
- Las pruebas pueden utilizar un reloj falso y ser completamente deterministas.

Este diseño también evita crear un temporizador host por cada solicitud encolada.

## Concurrencia

El núcleo no ejecutará trabajo concurrente. Mantendrá únicamente:

- Capacidad máxima actual.
- Número de solicitudes en ejecución.
- Solicitudes en espera.
- Orden de prioridad.
- Acciones que deben iniciarse cuando se libera capacidad.

Las llamadas a una misma instancia deberán serializarse. Un binding nativo podrá añadir sincronización externa cuando sea necesario.

El estado será normalmente **por instancia WebAssembly o por worker**. Coordinar un límite global entre workers o nodos requeriría un componente externo y queda fuera del núcleo común.

## Portabilidad

Se prevén varias capas de integración, sin obligar a que todas utilicen la misma ABI:

```text
controller-core        Lógica Rust independiente del host
controller-component   Interfaz WIT / WebAssembly Component Model
controller-proxy-wasm  Integración con hosts Proxy-Wasm
controller-cabi        ABI C opcional para bindings nativos
```

Cada gateway tiene capacidades diferentes. Para reproducir la cola del controlador debe permitir pausar y reanudar solicitudes. Los hosts que sólo permitan una decisión inmediata podrán integrar únicamente el modo de admisión `Allow | Reject`.

## Dependencias

El núcleo puede construirse con tipos de la biblioteca estándar de Rust:

- `BinaryHeap` para orden de prioridad y deadlines.
- `HashMap` para el estado de solicitudes.
- `Vec` o `VecDeque` para muestras e historial.
- Implementaciones locales y pequeñas de media, percentil y covarianza.

Los SDK oficiales o generadores de bindings podrán aparecer en crates adaptadores cuando sean necesarios. La política perseguida es:

> Cero dependencias externas en el algoritmo; dependencias mínimas, justificadas y aisladas en las integraciones.

No se plantea comenzar con `no_std`. Primero se estabilizarán el comportamiento y la frontera pública usando `std`; posteriormente podrá evaluarse `no_std + alloc` si un host real lo requiere.

## Persistencia

El estado inicial será exclusivamente en memoria:

- Cola de solicitudes.
- Estadísticas y ventanas.
- Integral y error previo del PID.
- Umbral de admisión.
- Límite de concurrencia.
- Historial del autoajuste.

Reiniciar una instancia descartará ese estado. Una futura restauración tendría que ser explícita y proporcionada por el host; las peticiones o callbacks en vuelo nunca formarían parte de un snapshot portable.

## Propiedades que se deben preservar

- Los valores de prioridad inferiores representan solicitudes más importantes.
- Las prioridades iguales mantienen orden FIFO.
- Una solicitud sólo realiza transiciones de estado válidas.
- Nunca se supera el límite de concurrencia decidido por el núcleo.
- Las solicitudes en cola pueden expirar sin ejecutar su trabajo.
- El shutdown cancela trabajo pendiente y no interrumpe trabajo ya iniciado.
- Los buckets de estadísticas describen periodos de ejecución coherentes.
- Los timestamps recibidos son monotónicos.
- El uso de memoria debe estar acotado mediante configuración y políticas explícitas.

## Estrategia de implementación

### 1. Especificación de comportamiento

Capturar mediante fixtures las secuencias y resultados de la implementación TypeScript existente:

- Decisiones de admisión.
- Orden de ejecución.
- Timeouts y cancelaciones.
- Cálculos PID.
- Percentiles y redondeos.
- Snapshots estadísticos.
- Ajuste de concurrencia.
- Shutdown.

### 2. Núcleo Rust

Implementar la máquina de estados sin WebAssembly, reloj real ni runtime asíncrono.

### 3. Pruebas de conformidad

Ejecutar las mismas trazas contra TypeScript y Rust, comparando acciones, thresholds, snapshots y límites de concurrencia.

### 4. Adaptador Node.js

Reconstruir una API idiomática basada en `Promise` para validar el diseño frente al proyecto original.

### 5. Primer gateway

Validar en un gateway concreto:

- Pausa y reanudación de solicitudes.
- Ciclo de vida de la instancia.
- Temporizadores.
- Modelo por worker.
- Límites de memoria.
- Recarga y shutdown.

### 6. Integraciones adicionales

Generalizar sólo después de comprobar qué garantías son comunes y cuáles pertenecen a cada host.

## Estado

- [x] Análisis inicial de viabilidad.
- [x] Definición preliminar de objetivos y restricciones.
- [ ] Contrato de comportamiento y fixtures de conformidad.
- [ ] Diseño de tipos y API Rust.
- [ ] Implementación del núcleo.
- [ ] Compilación WebAssembly.
- [ ] Adaptador Node.js.
- [ ] Integración con un primer gateway.

## Licencia

Este proyecto se distribuye bajo la [licencia MIT](LICENSE).
