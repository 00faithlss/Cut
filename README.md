# Cut

Cut guarda los últimos segundos de una ventana o pantalla como un archivo MP4. La aplicación para Windows 11 usa Rust. La interfaz se carga desde el mismo prototipo aprobado, con tiburones ASCII y tonos por pista de audio.

## Instalar en Windows

Para instalar Cut en Windows 11 de 64 bits, abre `Cut-<versión>-Setup-x64.exe`. El instalador crea un acceso en Inicio y prepara WebView2 si falta. Para actualizar o desinstalar, sal primero de Cut desde la bandeja. La desinstalación conserva los clips y el mezclador guardado.

## Compilar desde el código

Se necesitan Rust estable x64 y Microsoft Edge WebView2 Runtime. Con la toolchain GNU, Cargo también requiere `dlltool.exe` de MinGW-w64 en el PATH. El script detecta las herramientas de compilación instaladas en el directorio local de Cut.

```powershell
./scripts/build-windows.ps1 -Release
./target/release/cut.exe
```

El script pone `WebView2Loader.dll` junto al ejecutable. Mantén ambos juntos al moverlos.

Para construir el instalador, instala Inno Setup 6.3 o posterior y ejecuta `./scripts/build-installer.ps1`. El archivo queda en `target/installer/`.

### Actualizaciones

Desde Cut 0.2.0, la app consulta GitHub Releases al iniciar y cada seis horas. Si hay un instalador más nuevo, muestra **Ver actualización**. Puedes comprobarlo al instante desde **Ajustes → Buscar actualizaciones**. El enlace abre las notas y el instalador en GitHub, y puedes instalarlo después de salir de Cut. Los equipos necesitan acceso público al release para recibir el aviso.

Cada push a `main` hace que GitHub Actions pruebe Cut, incremente la versión del instalador y publique el release de Windows x64 con su SHA-256. El historial de cambios está en [CHANGELOG.md](CHANGELOG.md).

## Usar Cut

1. En **Clips**, elige una ventana o pantalla y ajusta la duración y el video. Puedes seleccionar hasta 120 FPS si tu GPU admite ese perfil.
2. Cut prepara el replay al abrir. La primera vez usa la pantalla principal. Después recupera tu selección y espera si la ventana elegida todavía no está abierta. En **Captura** puedes pausar o reanudar el replay. Pulsa **F9**, incluso con Cut en segundo plano, o **Guardar clip** para exportarlo a `Videos\Cut` de forma predeterminada. En **Clips → Carpeta de clips** puedes ver la ruta completa, abrirla o elegir otra para los próximos clips. Cut reproduce un sonido breve al guardar el MP4.
3. En **Ajustes**, elige el micrófono y configura mono, silencio y volumen. **Supresión de ruido** controla la reducción de ruido y la nivelación automática de voz. Viene activada y se puede cambiar durante la captura.
4. Al cerrar la ventana, Cut sigue capturando en la bandeja. Si activas **Minimizar al cerrar**, queda en la barra de tareas. Volver a abrir el ejecutable trae la misma ventana. Usa **Salir de Cut** para terminar la sesión. **Capturar cursor** incluye el puntero de Windows en el próximo replay.

El MP4 reproduce la mezcla de Cut de forma predeterminada. Cada aplicación capturada y el micrófono también conservan su pista AAC. La salida H.264 siempre llena el cuadro 16:9. Una fuente 4:3 se estira horizontalmente.

Mientras captura, Cut guarda el video comprimido del replay en archivos temporales de Windows. A 40 Mb/s, dos minutos de video equivalen aproximadamente a 600 MB de disco si el codificador alcanza ese bitrate. Cut libera los bloques antiguos cuando salen del replay y termina cualquier exportación que los esté usando.

El mezclador agrupa las pestañas y ventanas de cada aplicación en una pista. Las pistas aparecen automáticamente cuando producen audio, se ocultan tras tres segundos de silencio y desaparecen al cerrar su sesión. Cut conserva sus ajustes de volumen e inclusión en `%LOCALAPPDATA%\CutClipper\mezclador.json`. Los controles globales del micrófono están en **Ajustes**.

Los clips usan la fecha local del guardado. Una grabación del monitor se llama `Cut_07-03-2026.mp4` y una grabación de ventana puede llamarse `Cut_Valorant_07-03-2026.mp4`. Los siguientes clips con el mismo nombre reciben un número, como `_02`.

La captura de pantalla completa usa DXGI Desktop Duplication. Para una ventana se usa Windows Graphics Capture con el modo sin borde solicitado a Windows. D3D11 escala y convierte el video a NV12 en la GPU y Media Foundation lo codifica a H.264 por hardware. Si el codificador falla, Cut muestra el error. Los FPS configurados fijan la cadencia objetivo. La cantidad de imágenes distintas depende de las que entregue Windows y de la capacidad de la GPU para procesarlas a tiempo. El replay empieza en el último fotograma clave anterior al inicio solicitado, así que el archivo puede durar algo más que el intervalo configurado. El atajo predeterminado es F9. Puedes cambiarlo en **Ajustes → Atajo para guardar clips**.

[Abrir la maqueta en el navegador](docs/superpowers/concepts/cut-ascii.html) · [Ver la especificación](docs/superpowers/specs/2026-11-21-cut-design.md)
