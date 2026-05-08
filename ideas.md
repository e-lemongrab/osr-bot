Objetivo:
Crear una pequeña app en Rust que actúe como bot autorizado en un canal de Twitch.

Contexto:
- No es para spam, bot viewers ni actividad masiva.
- El canal ha dado permiso explícito.
- La cuenta usada será una cuenta de Twitch tipo viewer, no necesariamente una cuenta separada de bot.
- La frecuencia de uso será mínima.
- La app se desplegará en Kubernetes.
- El despliegue debe hacerse por CI/CD desde GitHub.
- El código y la configuración no sensible estarán en el repositorio de GitHub.
- Los secretos sensibles, especialmente el token OAuth de Twitch, no deben guardarse en claro en GitHub.

Comportamiento deseado:
- La app se conecta al chat de Twitch del canal objetivo.
- Escucha los mensajes del chat.
- Si detecta la frase:

"game restarting"

- Entonces espera 45 segundos.
- Después escribe en el chat:

"!play"

Protecciones necesarias:
- Solo debe actuar en el canal configurado.
- Solo debe reaccionar a la frase "game restarting".
- Debe evitar duplicados.
- Si ya hay un "!play" pendiente de enviarse, debe ignorar nuevos mensajes "game restarting".
- Después de enviar "!play", puede aplicar un cooldown corto, por ejemplo 60 segundos.
- No debe usarse un cooldown largo de 20 minutos, porque puede haber varios restarts reales durante integraciones o pruebas.

Lógica esperada:
1. Recibe un mensaje del chat.
2. Comprueba que viene del canal objetivo.
3. Convierte el mensaje a minúsculas.
4. Comprueba si contiene "game restarting".
5. Si no hay un "!play" pendiente:
   - marca pending_play = true
   - espera 45 segundos
   - envía "!play"
   - actualiza last_play_sent
   - marca pending_play = false
6. Si ya hay un "!play" pendiente, ignora el nuevo trigger.
7. Si el último "!play" fue hace menos de 60 segundos, ignora el trigger.

Ejemplo correcto:
12:00:00 -> game restarting
12:00:45 -> !play

Ejemplo con duplicados:
12:00:00 -> game restarting
12:00:05 -> game restarting
12:00:10 -> game restarting
12:00:45 -> !play

Resultado esperado:
Solo se envía un "!play".

Despliegue en Kubernetes:
- Crear un Deployment.
- Usar replicas: 1 obligatoriamente.
- No usar más de una réplica, porque podrían enviarse varios "!play" duplicados.
- La app puede ser un proceso long-running.
- No necesita base de datos inicialmente.
- El estado puede mantenerse en memoria.
- Logs por stdout.
- Recursos bajos.

CI/CD:
- El repositorio estará en GitHub.
- El pipeline debe construir la imagen Docker de la app Rust.
- El pipeline debe publicar la imagen en el registry configurado.
- El pipeline debe desplegar o actualizar la app en Kubernetes.
- El despliegue puede hacerse aplicando manifests Kubernetes o usando Helm.
- El token OAuth de Twitch no debe ir en el repositorio.
- El token debe existir como Kubernetes Secret o inyectarse desde el sistema de secretos del pipeline.
- El Deployment debe usar siempre replicas: 1.

Configuración sugerida:
- TWITCH_USERNAME
- TWITCH_OAUTH_TOKEN
- TWITCH_CHANNEL
- TRIGGER_TEXT=game restarting
- RESPONSE_TEXT=!play
- RESPONSE_DELAY_SECONDS=45
- MIN_COOLDOWN_SECONDS=60

Variables en GitHub/repo:
- Pueden ir en el repo las variables no sensibles.
- Por ejemplo:
  - TWITCH_CHANNEL
  - TRIGGER_TEXT
  - RESPONSE_TEXT
  - RESPONSE_DELAY_SECONDS
  - MIN_COOLDOWN_SECONDS
- No debe ir en claro:
  - TWITCH_OAUTH_TOKEN

Secreto:
- TWITCH_OAUTH_TOKEN debe ir como Kubernetes Secret, secreto del pipeline o sistema equivalente.

Resumen ultra corto:
Quiero una app Rust para Kubernetes que conecte al chat de Twitch con mi cuenta autorizada como viewer. Debe escuchar un canal concreto y, cuando detecte "game restarting", esperar 45 segundos y escribir "!play". Debe evitar duplicados: si ya hay un "!play" pendiente, debe ignorar nuevos triggers. Tras enviar, puede aplicar un cooldown corto de 60s. El despliegue debe ir por CI/CD desde GitHub: build de imagen Docker, push al registry y deploy/update en Kubernetes. Deployment con replicas: 1 para evitar doble envío. Config por variables; token OAuth como Secret, resto en repo/ConfigMap.
