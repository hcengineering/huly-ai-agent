# AI Agent Task Management System

## Overview
The agent actively monitors and processes incoming tasks while managing resource consumption.

## Task Types
1. **Direct Requests**
   - Immediate user queries or commands

2. **Mentions**
   - Tasks triggered by references in messages
   - Response to being tagged or mentioned

3. **Memory Processing**
   - Occurs during "sleep" mode
   - Analyzes and processes stored information
   - May generate new derivative tasks

4. **Learning Tasks**
   - Content study based on agent's profile
   - Self-improvement and knowledge acquisition
   - Can spawn additional learning objectives

## Resource Management
- Tasks are prioritized based on type and importance
- Each task execution requires coins (resource units)
- Daily resource allocation: 1000 coins
- Continuous balance monitoring and optimization


## Logging
To enable OpenTelemetry tracing, set the following environment variables:
```bash
export OTEL_EXPORTER_OTLP_ENDPOINT="https://api.uptrace.dev"
export OTEL_EXPORTER_OTLP_HEADERS="uptrace-dsn=https://<TOKEN>@api.uptrace.dev?grpc=4317"
export OTEL_EXPORTER_OTLP_COMPRESSION=gzip
export OTEL_EXPORTER_OTLP_METRICS_DEFAULT_HISTOGRAM_AGGREGATION=BASE2_EXPONENTIAL_BUCKET_HISTOGRAM
export OTEL_EXPORTER_OTLP_METRICS_TEMPORALITY_PREFERENCE=DELTA
```
