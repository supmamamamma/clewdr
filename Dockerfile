# 1. 使用 debian slim 作为基础镜像
FROM debian:bullseye-slim as builder

# 2. 设置环境变量
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# 3. 安装必要的依赖
RUN apt-get update && apt-get install -y --no-install-recommends \
    git build-essential cmake perl pkg-config libclang-dev musl-tools curl \
    && rm -rf /var/lib/apt/lists/*

# 4. 安装 Rust（选择安装稳定版）
RUN curl https://sh.rustup.rs -sSf | sh -s -- -y --default-toolchain stable

# 5. 设置工作目录
WORKDIR /app

# 6. 复制 Cargo.toml 和 Cargo.lock，进行依赖预构建
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release || true

# 7. 复制项目源码，并编译
COPY . .
RUN cargo build --release

# 8. 运行阶段，使用更小的镜像
FROM debian:bullseye-slim

# 9. 设置工作目录
WORKDIR /app

# 10. 复制编译好的二进制文件
COPY --from=builder /app/target/release/clewdr /usr/local/bin/clewdr

# 11. 运行程序
CMD ["clewdr"]