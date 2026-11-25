# Cut

**Replay instantáneo para Windows.** Cut guarda los últimos 15 segundos a 5 minutos de una ventana o pantalla en un MP4. Sigue capturando en la bandeja, así que puedes guardar lo que acaba de pasar con **F9** sin salir del juego.

[**Descargar Cut para Windows x64**](https://github.com/00faithlss/Cut/releases/latest) · [Historial de cambios](CHANGELOG.md)

> **Aviso de Windows.** El instalador de Cut no está firmado digitalmente. Al abrirlo, Microsoft Defender SmartScreen puede mostrar Windows protegió su PC. Descárgalo desde el enlace oficial de arriba. Cada release incluye un archivo SHA-256 para comprobar la descarga.

## Primer clip

1. Descarga y ejecuta `Cut-<versión>-Setup-x64.exe`. Necesitas Windows 11 de 64 bits y una GPU con codificación H.264 por hardware. El instalador prepara WebView2 si hace falta.
2. Abre Cut. La primera vez empieza a capturar la pantalla principal. En **Clips** puedes cambiar la fuente, la calidad y la duración del replay.
3. Pulsa **F9** o **Guardar clip**. Cut emite un sonido breve cuando termina de guardar el MP4 en `Videos\Cut`.

Puedes cambiar el destino en **Clips → Carpeta de clips**. Para reasignar F9, ve a **Ajustes → Atajo para guardar clips**, haz clic en la tecla y pulsa el nuevo atajo.

## Video y audio

- **Video.** Captura una ventana o el monitor completo. Elige entre 720p y 2160p, de 30 a 120 FPS y de 10 a 80 Mb/s. La cadencia real depende de la fuente, la GPU y la carga del equipo. La salida es 16:9 y las fuentes 4:3 se estiran para llenar el cuadro.
- **Mezclador.** Las aplicaciones aparecen cuando emiten audio y se ocultan al cerrar o tras tres segundos de silencio. Cut recuerda el volumen y si cada pista se incluye en los clips.
- **Pistas de audio.** En **Clips → Pistas de audio**, elige **Una pista (mezcla)** para reunir las fuentes incluidas en una sola pista AAC. **Varias pistas** conserva la mezcla y, además, las pistas de las aplicaciones y el micrófono. Los controles del micrófono están en **Ajustes**.

El replay mantiene el video comprimido en archivos temporales. Dos minutos a 40 Mb/s pueden ocupar unos 600 MB de disco. Cut libera los bloques al salir del historial cuando ninguna exportación los está usando.

## Actualizar o salir

Cut comprueba si hay una versión nueva al iniciar y cada seis horas. También puedes comprobarlo desde **Ajustes → Buscar actualizaciones**. Desde 0.2.2, **Actualización disponible → Descargar versión** descarga y verifica el instalador dentro de la app. Al terminar, Cut espera a los clips pendientes, instala la actualización y vuelve a abrirse automáticamente.

Cerrar la ventana deja la captura activa. Para detener Cut por completo antes de actualizar o desinstalar, usa **Salir de Cut** en el menú de la bandeja. La desinstalación conserva los clips y los ajustes del mezclador.

## Compilar desde el código

Necesitas Rust estable x64 y Microsoft Edge WebView2 Runtime. Si usas la toolchain GNU, agrega `dlltool.exe` de MinGW-w64 al PATH.

```powershell
./scripts/build-windows.ps1 -Release
./target/release/cut.exe
```

Mantén `WebView2Loader.dll` junto a `cut.exe`. Para crear el instalador, usa Inno Setup 6.3 o posterior y ejecuta `./scripts/build-installer.ps1`. El resultado queda en `target/installer/`.

La captura usa DXGI Desktop Duplication para el monitor y Windows Graphics Capture para ventanas. D3D11 escala y convierte el video a NV12 en la GPU y Media Foundation lo codifica a H.264 por hardware.

[Reportar un problema](https://github.com/00faithlss/Cut/issues) · [Especificación de diseño](docs/superpowers/specs/2026-11-21-cut-design.md)
