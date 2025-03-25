# 构建阶段
FROM rust:latest AS builder

# 安装系统依赖
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    git build-essential cmake perl pkg-config libclang-dev musl-tools && \
    rm -rf /var/lib/apt/lists/*

# 创建一个新项目
WORKDIR /usr/src/clewdr
RUN cargo new --bin .

# 复制 Cargo.toml 和 Cargo.lock
COPY Cargo.toml Cargo.lock ./
# 复制源代码
COPY src ./src

# 构建项目.  添加 musl target
RUN rustup target add x86_64-unknown-linux-musl
RUN cargo build --release --target=x86_64-unknown-linux-musl

# 运行阶段
FROM scratch

# 从构建阶段复制可执行文件
COPY --from=builder /usr/src/clewdr/target/x86_64-unknown-linux-musl/release/clewdr /usr/local/bin/clewdr

# 设置工作目录.  因为FROM scratch，根目录就是工作目录
WORKDIR /

# 运行程序（根据你的程序需求调整）
CMD ["/usr/local/bin/clewdr"]