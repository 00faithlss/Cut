# Changelog

## 0.2.0

- Replay móvil con historial de video comprimido en disco. Si se interrumpe la captura de pantalla completa, Cut vuelve a intentarlo y avisa cuando el historial se quedó sin video reciente.
- Captura, escalado y conversión a NV12 en GPU. Las imágenes nuevas se escriben directamente en las texturas del codificador H.264.
- Descarga de la interfaz al minimizar o cerrar a la bandeja. El selector de carpeta libera sus recursos al salir.
- Mezclador automático. Las pistas aparecen al emitir audio, se ocultan tras tres segundos de silencio y desaparecen al cerrar la sesión. El volumen y la inclusión se recuerdan por aplicación.
- Carpeta de clips visible, configurable y accesible desde Clips.
- Atajo editable desde Ajustes, con F9 como valor predeterminado. Haz clic en la tecla y pulsa el nuevo atajo.
- Instalador de Windows x64 y aviso de versiones nuevas a través de GitHub Releases.
