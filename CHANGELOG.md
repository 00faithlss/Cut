# Changelog

## 0.3.2 · Hotfix

- **Clips más fluidos.** Mejora la captura a 120 FPS cuando el juego mantiene la GPU ocupada.
- **Actualización a un clic del aviso.** Pulsa la notificación para ir a la descarga y ver su progreso; al terminar, Cut instala la actualización y vuelve a abrirse.

## 0.3.1 · Hotfix

- **Combinaciones para guardar clips.** Puedes asignar atajos como Ctrl + Mayús + S, Alt + Espacio o Ctrl + F9.
- **Pulsar para hablar con cualquier tecla.** Ahora puedes usar Alt, Ctrl, Mayús, Espacio, símbolos o el teclado numérico, solos o combinados.
- **Voz sin cortes.** Tu voz ya no se corta al pulsar Mayús o Ctrl mientras hablas, y puedes guardar un clip sin soltar la tecla.

## 0.3.0

- **Replay de 10 minutos.** Guarda los últimos momentos desde un historial comprimido en RAM, sin detener la captura.
- **Pulsar para hablar.** Asigna una tecla y tu voz entra al clip solo mientras la mantienes, también dentro del juego.
- **Atajos con el teclado numérico.** Números, Enter y operadores funcionan aunque Bloq Num esté desactivado.
- **Interfaz nativa.** Nuevo diseño con tiburones ASCII, temas claro y oscuro y controles más suaves, sin WebView2.
- **Mejoras de captura.** Colores corregidos, mejoras contra el audio duplicado y respaldo por CPU hasta 1080p a 30 FPS.

## 0.2.3

- Menos asignaciones de memoria durante la captura H.264 y la mezcla de audio. Al guardar un clip, la captura de video puede continuar mientras se copian las pistas de audio.
- Corrección del reloj de audio para sesiones que permanecen activas durante más de cuatro días.

- Compatibilidad con Windows 10 22H2 x64 actualizado. Se conservan el mezclador por aplicación, los modos de pistas, la captura por GPU y las actualizaciones desde Cut.
- La captura de ventanas puede iniciarse con el contorno que muestra Windows si el sistema no permite ocultarlo.

## 0.2.2

- Evita capturar dos veces procesos hijos que ya están incluidos en el loopback de su aplicación principal.
- La mezcla espera los bloques del micrófono que llegan después de otras aplicaciones. La exportación espera al audio pendiente antes de cerrar el MP4.
- Descarga de actualizaciones desde Cut con progreso y verificación SHA-256. Un solo clic en **Descargar versión** descarga, instala y reinicia la app. Los clips pendientes terminan antes de cerrar Cut.

## 0.2.1

- Opción para guardar clips con una sola pista de audio mezclada o con la mezcla y pistas separadas por fuente. El cambio se aplica al siguiente clip sin reiniciar la captura.

## 0.2.0

- Replay móvil con historial de video comprimido en disco. Si se interrumpe la captura de pantalla completa, Cut vuelve a intentarlo y avisa cuando el historial se quedó sin video reciente.
- Captura, escalado y conversión a NV12 en GPU. Las imágenes nuevas se escriben directamente en las texturas del codificador H.264.
- Descarga de la interfaz al minimizar o cerrar a la bandeja. El selector de carpeta libera sus recursos al salir.
- Mezclador automático. Las pistas aparecen al emitir audio, se ocultan tras tres segundos de silencio y desaparecen al cerrar la sesión. El volumen y la inclusión se recuerdan por aplicación.
- Carpeta de clips visible, configurable y accesible desde Clips.
- Atajo editable desde Ajustes, con F9 como valor predeterminado. Haz clic en la tecla y pulsa el nuevo atajo.
- Instalador de Windows x64 y aviso de versiones nuevas a través de GitHub Releases.
