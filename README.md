Assistant mode

Incoming events
- User activity (issues, chat mentions, etc)
- Direct message
- Periodic tasks (scheduled by IA agent)
- User presence

AI agent context:
- User profile from memory
- Memory items
- Notes (Responsibilities)
- Scheduled tasks


User activity -> update memory, update notes, schedule task
Direct message -> update memory, update responsibility, manage scheduler(periodic tasks or future task)
Periodic tasks -> update memory, update notes,
User presence -> can trigger scheduled task

User activity -> AssistantActivity task
Direct message -> AssistantChat task
User presence -> can trigger scheduled task

Task types:
- Follow chat - used only in employee mode
- AssistantChat - used only in personal assistant mode, use for direct message with user contains list of conversation with user, similar to chat with LLM
- AssistantActivity - used only in personal assistant mode, use for user activity, like issues, chat mentions, etc
- AssistantTask - used only in personal assistant mode, use for periodic tasks, postoned tasks or tasks should be triggered by user presence
- Sleep - system task for system memory consalidation
- Memory mantainance - system task for system memory cleanup

_update memory is automatically action for update memory bank_

## Logging
To enable OpenTelemetry tracing, set the following environment variables:
```bash
export OTEL_EXPORTER_OTLP_ENDPOINT="https://api.uptrace.dev"
export OTEL_EXPORTER_OTLP_HEADERS="uptrace-dsn=https://<TOKEN>@api.uptrace.dev?grpc=4317"
export OTEL_EXPORTER_OTLP_COMPRESSION=gzip
export OTEL_EXPORTER_OTLP_METRICS_DEFAULT_HISTOGRAM_AGGREGATION=BASE2_EXPONENTIAL_BUCKET_HISTOGRAM
export OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE=DELTA
```
