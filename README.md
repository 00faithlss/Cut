# Cut

**Replay instantáneo para Windows.** Cut graba en segundo plano los últimos momentos de tu pantalla o de una ventana. Cuando pasa algo que quieres conservar, pulsas **F9** y lo guarda en un MP4, sin salir del juego.

[**Descargar Cut para Windows x64**](https://github.com/00faithlss/Cut/releases/latest) · [Novedades](CHANGELOG.md) · [Reportar un problema](https://github.com/00faithlss/Cut/issues)

- Guarda de **15 segundos a 10 minutos** hacia atrás.
- El historial vive **comprimido en RAM**: no llena el disco de archivos temporales.
- **Mezclador de audio** con micrófono, aplicaciones y **pulsar para hablar**.
- Atajos con **cualquier combinación** de teclas, también del teclado numérico.
- Funciona en **Windows 10 desde la 1809** (incluidas LTSC 2019 y 2021) y en **Windows 11**.

> **Aviso de Windows.** El instalador no está firmado digitalmente, así que SmartScreen puede mostrar «Windows protegió su PC». Descárgalo solo desde el enlace de arriba. Cada versión incluye un archivo SHA-256 para comprobar la descarga.

## Primeros pasos

1. Descarga y ejecuta `Cut-<versión>-Setup-x64.exe`. No hace falta instalar nada más: ni WebView2 ni el Visual C++ Redistributable.
2. Abre Cut. Empieza a capturar la pantalla principal desde el primer momento.
3. Pulsa **F9** o **Guardar clip**. Un tono breve confirma que el clip quedó guardado en `Videos\Cut`.

Para cambiar la fuente, la calidad o la duración, entra en **Clips**. Para cambiar la carpeta, usa **Clips → Carpeta de clips**.

## Atajos

Los atajos funcionan con Cut en segundo plano o en la bandeja, también dentro del juego. Cámbialos en **Ajustes → Atajos**: haz clic en la tecla y pulsa la nueva.

| Acción | Qué teclas acepta |
| --- | --- |
| **Guardar clip** (F9 por defecto) | Una tecla F o del teclado numérico, solas o con modificadores, o cualquier tecla combinada con Ctrl o Alt: `Ctrl + Mayús + S`, `Alt + Espacio`, `Ctrl + F9`… |
| **Pulsar para hablar** (opcional) | Cualquier tecla, sola o combinada: `Alt`, `Ctrl`, `Mayús`, `Espacio`, símbolos, el teclado numérico… |

- Las letras solas no sirven para guardar, para que escribir no cree clips.
- Para usar solo `Alt` (o `Ctrl`, o `Mayús`) como tecla para hablar, púlsala y suéltala sin tocar otra.
- Con **pulsar para hablar** activo, tu voz entra al clip solo mientras mantienes la tecla. No se corta si pulsas Mayús o Ctrl mientras hablas, y puedes guardar un clip sin soltarla.
- Si eliges una tecla que ya usa el otro atajo, Cut te ofrece intercambiarlos o elegir otra.

## Calidad y duración

En **Clips** eliges:

- **Fuente:** pantalla completa o una ventana (ver [Ventana o pantalla completa](#ventana-o-pantalla-completa)).
- **Video:** de 720p a 2160p, de 30 a 120 FPS y de 10 a 80 Mb/s. La salida es 16:9; las fuentes 4:3 se estiran para llenar el cuadro.
- **Duración del replay:** 15 s, 30 s, 1, 2, 5 o 10 minutos.

Cut codifica por hardware con tu GPU. Si no lo consigue, usa el codificador de Windows por CPU, limitado a 1080p, 30 FPS y 20 Mb/s. La interfaz y el MP4 muestran siempre el perfil real.

### Cuánta memoria usa

El historial guarda el video ya comprimido, nunca imágenes sin comprimir. Lo que ocupa depende del bitrate y la duración:

| Duración | 20 Mb/s | 40 Mb/s | 80 Mb/s |
| --- | --- | --- | --- |
| 30 segundos | ~75 MB | ~150 MB | ~300 MB |
| 2 minutos | ~300 MB | ~600 MB | ~1,2 GB |
| 10 minutos | ~1,5 GB | ~3 GB | ~6 GB |

Son cifras aproximadas del video; el audio y la propia app suman algo más. La parte que sale de la ventana del replay se libera en cuanto ningún guardado la necesita.

Al pulsar F9 se guarda lo que haya en esa ventana: con 2 minutos elegidos, aproximadamente los dos últimos minutos, aunque Cut lleve horas abierto. El inicio se ajusta a un fotograma clave, así que puede añadir un instante más. Si acabas de abrir Cut, guarda lo que haya disponible. La captura sigue mientras se escribe el MP4.

## Audio

- **Mezclador.** En **Captura** ves cada fuente con su interruptor para incluirla en el clip y su volumen. Cut recuerda ambos por fuente.
  - En Windows 11, cada aplicación aparece al emitir sonido y se oculta al cerrarse o tras tres segundos de silencio.
  - En Windows 10, **Audio del sistema** reúne lo que suena en el dispositivo de salida predeterminado y no permite separar aplicaciones.
- **Micrófono.** Tiene su propia pista. En **Ajustes → Micrófono** eliges el dispositivo, el volumen de grabación y las opciones **Mono**, **Silenciar** y **Supresión** de ruido.
- **Pistas del MP4.** En **Clips → Pistas de audio**:
  - **Una pista (mezcla):** todo lo incluido en una sola pista AAC.
  - **Varias pistas:** la mezcla más una pista por aplicación y otra para el micrófono, útil para editar después. Solo la mezcla suena por defecto, para no oír el audio duplicado.

## Ventana o pantalla completa

| | Pantalla completa | Una ventana |
| --- | --- | --- |
| Windows 10 / LTSC | ✅ | ❌ |
| Windows 11 | ✅ | ✅ sin borde, si Windows lo permite |
| Borde amarillo de Windows | Nunca | No; Cut no inicia la captura si no puede evitarlo |

**Pantalla completa** es la opción recomendada en Windows 10, y la que nunca muestra el borde amarillo. La captura de una ventana necesita Windows 11. Si otra aplicación captura la misma ventana, Windows puede mostrar su propio indicador.

## Compatibilidad

| Función | Requisito |
| --- | --- |
| Cut | Windows 10 1809 o posterior de 64 bits (incluidas LTSC 2019 y 2021) o Windows 11 (también LTSC 2024) |
| Capturar pantalla completa | GPU compatible con Direct3D 11 |
| Codificar por hardware | GPU con codificador H.264; si no, se usa la CPU hasta 1080p/30 FPS |
| Capturar una ventana | Windows 11 |
| Audio separado por aplicación | Windows 11; Windows 10 usa el audio del sistema |

Las ediciones LTSB 2015/2016, las primeras versiones de Windows 10 y Windows de 32 bits no son compatibles. El funcionamiento depende también de la GPU y sus controladores; no se han probado todas las combinaciones.

## Bandeja, actualizaciones y desinstalación

- **Cerrar la ventana** deja Cut capturando en la bandeja. Desde su menú puedes abrir Cut, pausar o reanudar la captura, guardar un clip o **Salir de Cut**. Si prefieres que la X minimice la ventana, activa **Ajustes → Comportamiento → Minimizar al cerrar**.
- **Actualizaciones.** Cut busca versiones nuevas al iniciar y cada seis horas; también puedes buscarlas en **Ajustes → Acerca de**. Al aceptar, descarga y verifica el instalador, espera a que terminen los clips pendientes, se actualiza y vuelve a abrirse. Después de cada actualización, **Novedades** te cuenta qué cambió.
- **Desinstalar.** Usa antes **Salir de Cut** en la bandeja. La desinstalación conserva tus clips y los ajustes del mezclador.

En **Ajustes** también puedes iniciar Cut con Windows, capturar el cursor, apagar el sonido al guardar y elegir el tema oscuro, claro o el del sistema. Los tiburones del fondo se pueden pausar y se detienen solos al minimizar Cut.

## Preguntas frecuentes

**¿Por qué no puedo capturar una ventana?** En Windows 10 y LTSC no está disponible: usa **Pantalla completa**. En Windows 11, Cut no inicia la captura de una ventana si Windows no le permite hacerlo sin el borde amarillo.

**¿Por qué no veo cada aplicación en el mezclador?** Separar el audio por aplicación requiere Windows 11. En Windows 10 aparece **Audio del sistema**.

**¿Por qué un clip antiguo se ve lavado en VLC?** Desde 0.3.0, Cut indica en el MP4 cómo interpretar los colores para que los reproductores los muestren bien. Los clips guardados antes no cambian. El contenido HDR no está validado.

**¿Cut gasta mucha memoria?** Depende de la duración y el bitrate que elijas (ver [Cuánta memoria usa](#cuánta-memoria-usa)). Si necesitas reducirla, baja uno de los dos.

## Compilar desde el código

Necesitas Rust estable con la toolchain MSVC y el SDK de Windows (Visual Studio Build Tools). `rust-toolchain.toml` elige la toolchain automáticamente.

```powershell
./scripts/get-fonts.ps1            # una vez: fuentes de la interfaz (licencia SIL OFL)
./scripts/build-windows.ps1 -Release
./target/release/Cut.exe
```

`Cut.exe` funciona solo, sin DLL al lado. Para crear el instalador, instala Inno Setup 6.3 o posterior y ejecuta `./scripts/build-installer.ps1` desde PowerShell 7. El resultado queda en `target/installer/`.

### Cómo funciona

- **Interfaz:** nativa en Rust con [GPUI](https://www.gpui.rs/), sin navegador incrustado.
- **Captura:** DXGI Desktop Duplication para el monitor y Windows Graphics Capture para ventanas.
- **Video:** Direct3D 11 escala y convierte a NV12 BT.709 de rango limitado en la GPU. Media Foundation codifica a H.264 por hardware o, como respaldo, con el [codificador H.264 de Windows](https://learn.microsoft.com/en-us/windows/win32/medfound/h-264-video-encoder) por CPU. El rango, las primarias y la transferencia se declaran en el MP4.
- **Audio:** WASAPI, con [loopback por proceso](https://learn.microsoft.com/en-us/samples/microsoft/windows-classic-samples/applicationloopbackaudio-sample/) en Windows 11 y [loopback del sistema](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording) en Windows 10.
- **Replay:** H.264 y AAC comprimidos en RAM dentro de una ventana móvil. Solo se escribe al disco el MP4 al guardar.

Los límites de compatibilidad vienen de las APIs de Microsoft para [capturar ventanas](https://learn.microsoft.com/en-us/windows/win32/api/windows.graphics.capture.interop/nf-windows-graphics-capture-interop-igraphicscaptureiteminterop-createforwindow) y [controlar el cursor](https://learn.microsoft.com/en-us/uwp/api/windows.graphics.capture.graphicscapturesession.iscursorcaptureenabled). La interfaz necesita además DirectWrite de Windows 10 Creators Update o posterior.
