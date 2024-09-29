#== Second stage: 
FROM debian:bookworm-slim
LABEL description="EightFish:dtomcat"

WORKDIR /eightfish

RUN mkdir -p /eightfish/tmp_configs
RUN mkdir -p /eightfish/wasm_files

COPY ./spin /usr/local/bin
COPY ./target/release/dtomcat /usr/local/bin
COPY ./spin_tmpl.toml /eightfish/
