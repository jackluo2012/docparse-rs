# docparse-rs REST sidecar — multi-stage build.
# Pure-Rust single binary; model tiers (OCR/layout/UniRec, ~780MB) are NOT
# baked in by default — mount them at /app/models at runtime, or build with
# `--build-arg INSTALL_MODELS=1` for a fully self-contained image.
# syntax=docker/dockerfile:1
# REGISTRY: base-image registry prefix. Default empty = Docker Hub. For mirror-only
# networks set e.g. --build-arg REGISTRY=docker.1ms.run/library/ (compose passes it).
ARG REGISTRY=

FROM ${REGISTRY}rust:1-bookworm AS builder
WORKDIR /build
COPY . .
RUN cargo build --release --bin docparse

FROM ${REGISTRY}debian:bookworm-slim
# ca-certificates: outbound HTTPS for model downloads and the optional VLM
# enhancer (--vlm-url). curl: container HEALTHCHECK against /healthz.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /app
COPY --from=builder /build/target/release/docparse /usr/local/bin/docparse

# INSTALL_MODELS=1 bakes the three default neural tiers into the image at
# build time (ppocr-v6 ~7MB + DocLayout-YOLO ~75MB + UniRec ~700MB, all
# Apache-2.0): the image then serves ocr/layout/table/formula/transcribe with
# zero setup. Default 0 keeps the image ~100MB; mount models at /app/models.
# PP-DocLayoutV2 is not baked — it needs a one-time onnxsim static-ize step.
ARG INSTALL_MODELS=0
RUN if [ "$INSTALL_MODELS" = "1" ]; then \
        docparse fetch-models ppocr-v6 --dir /app/models \
        && docparse fetch-models layout --dir /app/models \
        && docparse fetch-models unirec --dir /app/models; \
    fi
ENV DOCPARSE_OCR_MODELS=/app/models/ppocr-v6 \
    DOCPARSE_LAYOUT_MODEL=/app/models/layout/doclayout_yolo.onnx \
    DOCPARSE_UNIREC_MODELS=/app/models/unirec

EXPOSE 8642
# 0.0.0.0 so other containers on the compose network can reach it (no auth —
# keep it on a private network, never publish the port to the host/internet).
# shell form (not exec) so the ENV paths and the optional VLM_* env expand;
# `exec` re-spawns docparse as PID 1 so SIGTERM reaches it for `docker stop`.
# VLM_URL+VLM_MODEL (compose/docker run -e) enable the VLM enhancer at startup.
ENTRYPOINT ["sh", "-c", "extra=''; [ -n \"$VLM_URL\" ] && extra=\"--vlm-url $VLM_URL --vlm-model ${VLM_MODEL:-}\"; [ -n \"${VLM_API_KEY:-}\" ] && extra=\"$extra --vlm-api-key $VLM_API_KEY\"; exec docparse serve --host 0.0.0.0 --port 8642 --ocr-models \"$DOCPARSE_OCR_MODELS\" --layout-model \"$DOCPARSE_LAYOUT_MODEL\" --unirec-models \"$DOCPARSE_UNIREC_MODELS\" --cache-dir /app/cache $extra"]
