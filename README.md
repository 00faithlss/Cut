# Cut

Cut guarda los últimos segundos de una ventana o pantalla como un archivo MP4. La aplicación para Windows 11 usa Rust. La interfaz se carga desde el mismo prototipo aprobado, con tiburones ASCII y tonos por pista de audio.

## Ejecutar en Windows

Se necesitan Rust estable x64 y Microsoft Edge WebView2 Runtime. Con la toolchain GNU, Cargo también requiere `dlltool.exe` de MinGW-w64 en el PATH. El script detecta las herramientas de compilación instaladas en el directorio local de Cut.

```powershell
./scripts/build-windows.ps1 -Release
./target/release/cut.exe
```

El script pone `WebView2Loader.dll` junto al ejecutable. Mantén ambos juntos al moverlos.

1. En **Clips**, elige una ventana o pantalla y ajusta la duración y el video. Puedes seleccionar hasta 120 FPS si tu GPU admite ese perfil.
2. Cut prepara el replay al abrir. La primera vez usa la pantalla principal. Después recupera tu selección y espera si la ventana elegida todavía no está abierta. En **Captura** puedes pausar o reanudar el replay. Pulsa **F9**, incluso con Cut en segundo plano, o **Guardar clip** para exportarlo a `Videos\Cut`. Cut reproduce un sonido breve al guardar el MP4.
3. En **Ajustes**, elige el micrófono y configura mono, silencio y volumen. **Supresión de ruido** controla la reducción de ruido y la nivelación automática de voz. Viene activada y se puede cambiar durante la captura.
4. Al cerrar la ventana, Cut sigue capturando en la bandeja. Si activas **Minimizar al cerrar**, queda en la barra de tareas. Volver a abrir el ejecutable trae la misma ventana. Usa **Salir de Cut** para terminar la sesión. **Capturar cursor** incluye el puntero de Windows en el próximo replay.

El MP4 reproduce la mezcla de Cut de forma predeterminada. Cada aplicación capturada y el micrófono también conservan su pista AAC. La salida H.264 siempre llena el cuadro 16:9. Una fuente 4:3 se estira horizontalmente.

El mezclador agrupa las pestañas y ventanas de cada aplicación en una pista. **Buscar pistas** actualiza las aplicaciones con audio abierto. Cut guarda las aplicaciones conocidas y sus ajustes en `%LOCALAPPDATA%\CutClipper\mezclador.json` y los muestra al abrir. Si la lista es larga, desplázate dentro del mezclador.

Los clips usan la fecha local del guardado. Una grabación del monitor se llama `Cut_07-03-2026.mp4` y una grabación de ventana puede llamarse `Cut_Valorant_07-03-2026.mp4`. Los siguientes clips con el mismo nombre reciben un número, como `_02`.

La captura de pantalla completa usa DXGI Desktop Duplication. Para una ventana se usa Windows Graphics Capture con el modo sin borde solicitado a Windows. El adaptador gráfico codifica el video mediante Media Foundation. Si el codificador por hardware falla, Cut muestra el error. El replay empieza en el último fotograma clave anterior al inicio solicitado, así que el archivo puede durar algo más que el intervalo configurado. El atajo y la carpeta de exportación son fijos en esta versión.

[Abrir la maqueta en el navegador](docs/superpowers/concepts/cut-ascii.html) · [Ver la especificación](docs/superpowers/specs/2026-11-21-cut-design.md)
