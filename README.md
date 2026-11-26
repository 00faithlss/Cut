# Cut

**Replay instantáneo para Windows.** Cut guarda los últimos 15 segundos a 10 minutos de una ventana o pantalla en un MP4. Sigue capturando en la bandeja, así que puedes guardar lo que acaba de pasar con **F9** sin salir del juego.

[**Descargar Cut para Windows x64**](https://github.com/00faithlss/Cut/releases/latest) · [Historial de cambios](CHANGELOG.md)

> **Aviso de Windows.** El instalador de Cut no está firmado digitalmente. Al abrirlo, Microsoft Defender SmartScreen puede mostrar Windows protegió su PC. Descárgalo desde el enlace oficial de arriba. Cada release incluye un archivo SHA-256 para comprobar la descarga.

## Primer clip

1. Descarga y ejecuta `Cut-<versión>-Setup-x64.exe`. Necesitas Windows 10 versión 1809 o posterior (incluidas las ediciones LTSC 2019 y 2021) o Windows 11, de 64 bits, con captura y conversión D3D11 disponibles. Cut usa H.264 por hardware cuando puede; si no logra abrirlo, usa el encoder H.264 de Windows por CPU hasta 1080p/30 FPS. Desde 0.3.0, Cut ya no necesita WebView2 ni el Visual C++ Redistributable.
2. Abre Cut. La primera vez empieza a capturar la pantalla principal. En **Clips** puedes cambiar la fuente, la calidad y la duración del replay.
3. Pulsa **F9** o **Guardar clip**. Cut emite un sonido breve cuando termina de guardar el MP4 en `Videos\Cut`.

Puedes cambiar el destino en **Clips → Carpeta de clips**. Para reasignar F9, ve a **Ajustes → Atajos**, haz clic en la tecla y pulsa el nuevo atajo. Ahí también puedes asignar una tecla para **pulsar para hablar**; acepta teclas del teclado numérico.

## Video y audio

- **Video.** Captura una ventana o el monitor completo. Con encoder por hardware, elige entre 720p y 2160p, de 30 a 120 FPS y de 10 a 80 Mb/s. El respaldo por CPU limita el perfil efectivo a 1080p/30 FPS y 20 Mb/s, sin aumentar fuentes configuradas a 720p. La interfaz y el MP4 muestran el perfil real. La cadencia depende de la fuente y la carga del equipo. La salida es 16:9 y las fuentes 4:3 se estiran para llenar el cuadro. Windows puede mostrar un contorno alrededor de la ventana que se está capturando.
- **Mezclador.** En Windows 11, las aplicaciones aparecen cuando emiten audio y se ocultan al cerrar o tras tres segundos de silencio. En Windows 10, **Audio del sistema** reúne las aplicaciones del dispositivo de salida predeterminado; no permite separarlas ni excluirlas individualmente. El micrófono mantiene su propia pista. Cut recuerda el volumen y la inclusión de cada fuente.
- **Pistas de audio.** En **Clips → Pistas de audio**, elige **Una pista (mezcla)** para reunir las fuentes incluidas en una sola pista AAC. **Varias pistas** conserva la mezcla y, además, las pistas de las aplicaciones y el micrófono. Se marcan como alternativas: solo la mezcla está habilitada por defecto, para evitar sumarlas al reproducir. En Windows 11, excluir un hijo también bloquea una raíz de captura que lo incluya y muestra el motivo; las demás instancias independientes siguen disponibles. Los controles del micrófono están en **Ajustes**.

El replay conserva **H.264 y AAC comprimidos en RAM**, en una ventana móvil. Con dos minutos seleccionados, F9 guarda aproximadamente los últimos dos minutos del instante de pulsación, aunque Cut lleve horas abierto; el inicio se alinea a un fotograma clave y puede añadir hasta un GOP. Si acabas de abrir Cut, guarda lo disponible. No crea un video acumulado desde el inicio ni escribe el historial en archivos temporales. Solo escribe el MP4 al guardar y continúa capturando mientras exporta.

Dos minutos a 40 Mb/s pueden ocupar unos **600 MB de RAM**; diez minutos, unos 3 GB, más audio y estructuras. No se almacenan imágenes crudas. Los snapshots comparten los mismos bytes del historial, que se liberan cuando salen de la ventana y ninguna exportación los necesita. La interfaz muestra el tamaño del video comprimido y la estimación según el perfil; ese contador no es la memoria total de la aplicación.

## Compatibilidad de Windows

Para grabar sin líneas amarillas, elige **Clips → Fuente → Pantalla completa**: esta ruta usa DXGI y no crea el borde de Windows Graphics Capture. La captura de una ventana exige Windows 11 y permiso de captura sin borde; Cut no la inicia si no puede confirmarlos. Otra aplicación que capture la misma ventana puede exigir su propio indicador del sistema. En Windows 10/LTSC, usa pantalla completa.

Los clips SDR nuevos se convierten de RGB completo a NV12 BT.709 limitado. El rango, las primarias y la transferencia se declaran también al encoder y en el MP4, para evitar que un reproductor como VLC interprete el video con contraste o saturación incorrectos. Este cambio no modifica los clips ya guardados; HDR no está validado por esta corrección.

El mínimo del ejecutable nativo es **Windows 10 1809 x64**. Incluye LTSC 2019/2021 y Windows 11, también LTSC 2024, sujeto al soporte del dispositivo y del controlador gráfico. LTSC 2015/2016 y Windows de 32 bits no están admitidos por esta compilación. No se ha validado una matriz física de todas las versiones.

| Función | Requisito |
| --- | --- |
| Capturar pantalla completa | Windows 10 1809 o posterior; captura y conversión D3D11 |
| H.264 de respaldo por CPU | Encoder H.264 de Windows disponible; máximo 1080p/30 FPS |
| Capturar una ventana sin borde | Windows 11 y permiso de captura sin borde confirmado |
| Capturar sin borde en Windows 10/LTSC | Pantalla completa mediante DXGI |
| Audio separado por aplicación | Windows 11; Windows 10 usa audio del sistema |

Si una función no existe, Cut indica la limitación y permite elegir pantalla completa. Si hay exclusiones de aplicaciones guardadas y todavía no has elegido cómo incluir el audio del sistema, esa pista empieza desactivada para conservar tu decisión anterior.

Estos límites provienen de las APIs de Microsoft: [captura de ventanas](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow), [control del cursor](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.iscursorcaptureenabled), [loopback por proceso](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/) y [loopback del sistema](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording). GPUI también necesita DirectWrite de Windows 10 Creators Update; esta interfaz no puede ejecutarse en las primeras ediciones de Windows 10.

## Actualizar o salir

Cut comprueba si hay una versión nueva al iniciar y cada seis horas. También puedes comprobarlo desde **Ajustes → Buscar actualizaciones**. Desde 0.2.2, **Actualización disponible → Descargar versión** descarga y verifica el instalador dentro de la app. Al terminar, Cut espera a los clips pendientes, instala la actualización y vuelve a abrirse automáticamente.

Cerrar la ventana deja la captura activa. Para detener Cut por completo antes de actualizar o desinstalar, usa **Salir de Cut** en el menú de la bandeja. La desinstalación conserva los clips y los ajustes del mezclador.

## Compilar desde el código

Necesitas Rust estable con la toolchain MSVC y el SDK de Windows (Visual Studio Build Tools); `rust-toolchain.toml` la selecciona sola. Ejecuta una vez `./scripts/get-fonts.ps1` para incrustar las fuentes de la interfaz.

```powershell
./scripts/build-windows.ps1 -Release
./target/release/Cut.exe
```

`Cut.exe` funciona solo, sin DLL al lado. Para crear el instalador, usa Inno Setup 6.3 o posterior y ejecuta `./scripts/build-installer.ps1`. El resultado queda en `target/installer/`.

La captura usa DXGI Desktop Duplication para el monitor y Windows Graphics Capture para ventanas. D3D11 escala y convierte el video a NV12 en la GPU. Media Foundation lo codifica a H.264 por hardware; el respaldo por CPU lee únicamente la superficie NV12 reducida y usa el [encoder H.264 de Windows](https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder).

[Reportar un problema](https://github.com/00faithlss/Cut/issues)
