# Cut para Windows

## Objetivo

Guardar lo que acaba de pasar en el juego con una tecla, en alta calidad y con el menor impacto posible sobre la partida. El usuario elige cuánto tiempo conservar, qué imagen capturar y con qué calidad. El micrófono y las aplicaciones quedan en pistas identificables e independientes.

## Experiencia

- La captura se activa y pausa desde la bandeja. La ventana principal muestra si el búfer está activo, cuántos segundos hay disponibles y qué codificador se está usando.
- Un botón «Guardar clip» y un atajo global configurable guardan el intervalo anterior a la pulsación. Se sigue capturando mientras se escribe el clip. Si el búfer se acaba de activar, se guarda el tramo disponible y se indica su duración.
- El usuario elige una ventana de juego o un monitor. La ventana se selecciona manualmente en la primera versión. Cuando la fuente se cierra o se confirma que la captura se interrumpió, se informa y se conserva el replay hasta el último fotograma válido. Al reanudar se inicia un búfer nuevo. La captura nunca cambia al monitor por su cuenta.
- Los ajustes incluyen duración, carpeta, atajo, resolución, FPS y bitrate. Propuesta inicial de 30 s, 1080p, 60 FPS y H.264 a 20 Mb/s cuando el equipo lo admita. El menú ofrece combinaciones de 720p, 1080p, 1440p y 4K a 30 o 60 FPS según las capacidades detectadas. El lienzo se fija al activar el perfil. Si la fuente se achica o cambia de proporción, se centra en ese mismo lienzo con bandas y sin ampliar la imagen por encima de su resolución nativa.
- Las pistas de aplicaciones se detectan automáticamente y se nombran por aplicación. Se puede excluir una app del replay. Al excluirla se descarta su audio del búfer y se deja de capturar hasta volver a habilitarla. Los clips ya guardados conservan sus pistas. El modo de video se controla independientemente del audio. Una app que arranca a mitad de un clip conserva su posición temporal, con silencio para el tramo anterior a su entrada.
- El micrófono tiene controles globales de dispositivo, volumen de grabación, silencio, mono y medidor de nivel. El modo mono permite centrar un canal presente solamente a la izquierda o derecha. Al silenciar se deja de incorporar audio nuevo del micrófono al búfer.
- En un clip guardado, el único control de edición es el volumen de la pista del micrófono. El usuario puede escuchar la mezcla de previsualización y guardar una copia que mantiene intactas las demás pistas. Los cambios posteriores a los ajustes globales afectan clips nuevos, sin modificar los existentes.
- Los dos controles de volumen del micrófono se muestran como porcentajes. La escala propuesta va de 0% (silencio) a 200% (doble de ganancia), con 100% como nivel original y una acción para restablecerlo.
- En Captura, cada fila del mezclador reúne la casilla para incluir la pista y su volumen en porcentaje. Al excluir y volver a incluir una fuente se recupera su volumen. La fila del micrófono comparte el valor global de Ajustes. El botón «Buscar pistas» vuelve a consultar las fuentes y agrega las nuevas sin duplicar pistas ni reiniciar niveles.

## Enfoques considerados

- **Rust nativo, elegido por el usuario.** Captura y codificación mediante las API de Windows, con las imágenes en la GPU cuando el hardware lo permita. Permite controlar la memoria y el trabajo que continúa en segundo plano. Tiene más trabajo inicial de integración y pruebas por GPU.
- **C# y FFmpeg.** Permite prototipar más rápido y la captura de monitor ya se comprobó en este equipo. También puede usar hardware si se valida la ruta de GPU, pero añade un binario externo y limita el control directo sobre el ciclo de captura y el búfer. La plantilla anterior se conserva como referencia durante el diseño.
- Cut es un proyecto independiente. Clippy es referencia para el flujo de replay, la bandeja y la codificación por hardware.

## Diseño técnico propuesto

**Video y codificación.** Windows 11 x64 como punto de partida. Windows Graphics Capture para ventana o monitor, D3D11 para conversión y escalado en GPU, y Media Foundation para H.264 con codificador por hardware. La app comprueba las capacidades reales del codificador y muestra un error claro si el perfil no está disponible. La alternativa por CPU queda desactivada por defecto y se ofrece explícitamente con un perfil de menor carga. HEVC, AV1 y HDR quedan para una fase posterior, según soporte y mediciones.

**Reloj, retención y exportación.** Video y audio se alinean contra el reloj monotónico de Windows. Un búfer circular guarda paquetes de video comprimido y audio con sus marcas temporales. Se limita por tiempo y memoria, con estimación visible para la duración y bitrate elegidos. Por ejemplo, 60 s a 20 Mb/s representan unos 150 MB de video antes del audio y otros gastos. Se deja margen para conservar el fotograma clave anterior al inicio solicitado. El guardado fija el intervalo y comparte referencias a esos paquetes mientras la captura sigue. La escritura tiene una cola acotada y usa un temporal antes de publicar el archivo terminado. El corte visible debe corresponder al intervalo solicitado con una tolerancia de un fotograma a cada extremo. Validarlo con clips que empiezan entre fotogramas clave y en los reproductores elegidos. Si hace falta recodificar, probar primero recodificar solo el tramo inicial durante el guardado y medir su costo. Los cambios de perfil que requieran reiniciar el codificador indican que se recargará el búfer.

**Audio.** WASAPI para el micrófono y captura por proceso para las aplicaciones que Windows pueda identificar. La detección sigue sesiones de audio y evita sumar dos veces el mismo árbol de procesos. Un navegador puede tener una pista para el navegador completo. Se validan apps con procesos auxiliares, reconexión de dispositivos y audio iniciado durante la captura. Una fuente inaccesible se señala en la app. El procesamiento global del micrófono se aplica al entrar al búfer, con detección de saturación. El ajuste por clip actúa sobre la pista ya grabada y se limita al volumen.

**Formato.** MKV multipista con H.264 y audio AAC como formato propuesto para la primera versión. Cada pista lleva nombre. El visor de Cut reproduce las fuentes juntas para previsualizar el volumen. La copia con volumen ajustado reutiliza el video y las demás pistas sin recodificarlas. Un MP4 con mezcla para compartir se considera después de confirmar el flujo de edición.

## Cómo comprobaremos la primera versión

1. Guardar clips configurados de 15, 30 y 60 segundos mientras corre un juego. Medir duración, tiempo de guardado, sincronía y comportamiento con el búfer recién activado.
2. Comparar captura de ventana y monitor con otras ventanas superpuestas, cambio de foco, cambio de tamaño y cierre del juego. Probar al menos un juego en modo ventana sin bordes y registrar el resultado de fullscreen exclusivo por juego.
3. Hacer sonar juego, Discord, Spotify y micrófono a la vez. Verificar nombres, aislamiento, apps que se suman a mitad del clip y silencio tras desactivar el micrófono. Confirmar que un micro de un solo canal se oye centrado en mono.
4. Ajustar el volumen del micrófono en una copia y verificar que el clip original, el video y las otras pistas se conservan. Comprobar con ffprobe que todas las pistas existen y se pueden identificar, reproducirlas por separado en VLC y validar su uso en un editor externo que acepte MKV. Confirmar el editor habitual del usuario si requiere compatibilidad específica.
5. Medir el mismo tramo de un juego con Cut apagado y con Cut a 1080p60 por hardware. Registrar FPS promedio, 1% low, CPU, GPU, RAM y cuadros de captura perdidos. Repetir al guardar y con los perfiles de mayor calidad que admita el equipo. Usar la RTX 3050 detectada en este equipo como primera candidata de referencia y registrar también CPU, juego y versión del controlador. Verificar resolución, FPS efectivo y bitrate del archivo, y revisar una escena con movimiento rápido y texto pequeño. Validar que la memoria alcanza un techo con captura prolongada. Definir el presupuesto de impacto a partir de la máquina de referencia y documentar las mediciones por perfil.

## Orden de validación

1. Probar la ruta de GPU y medir el primer replay de alta calidad con búfer acotado.
2. Validar captura automática por aplicación, sincronía y micrófono global.
3. Integrar bandeja, atajo, ajustes, guardado MKV y edición de volumen por clip.
4. Probar fallos recuperables y validar el rendimiento y la compatibilidad con juegos reales.

## Referencias

- [Clippy](https://github.com/santiremedi/clippy)
- [Captura de pantalla y ventanas de Windows](https://learn.microsoft.com/en-us/windows/apps/develop/media-authoring-processing/screen-capture)
- [Media Foundation y superficies Direct3D](https://learn.microsoft.com/en-us/windows/win32/medfound/direct3d-aware-mfts)
- [Captura WASAPI por proceso](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/)
