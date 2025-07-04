#FROM rust:1-alpine3.19
FROM rust:1.86 AS builder

# This is important, see https://github.com/rust-lang/docker-rust/issues/85
ENV RUSTFLAGS="-C target-feature=-crt-static"

# dependecies for building
#RUN apk add --no-cache musl-dev
# need for build openssl
#RUN apk add perl make
# need for build rdkafka
#RUN apk add gcc cmake libressl-dev
#RUN apk add build-base cmake
#RUN ln -s /usr/bin/musl-gcc /usr/bin/musl-g++

RUN apt-get update && apt-get install -y cmake curl && rm -rf /var/lib/apt/lists/*

# set the workdir and copy the source into it
WORKDIR /app
COPY ./ /app
# do a release build
RUN cargo build --release
RUN strip target/release/huly-ai-agent

# use a plain alpine image, the alpine version needs to match the builder
# FROM alpine:3.19
FROM debian:12-slim

# if needed, install additional dependencies here
#RUN apk add --no-cache libgcc
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

# copy the binary into the final image
COPY --from=builder /app/target/release/huly-ai-agent .
# set the binary as entrypoint
ENTRYPOINT ["/huly-ai-agent"]
