# 1. 使用 debian slim 作为基础镜像
FROM debian:bullseye-slim AS builder

# 2. 设置环境变量
ENV RUSTUP_HOME=/usr/local/rustup \
    CARGO_HOME=/usr/local/cargo \
    PATH=/usr/local/cargo/bin:$PATH

# 3. 安装必要的系统依赖
RUN apt-get update && apt-get install -y --no-install-recommends \
    git build-essential cmake perl pkg-config libclang-dev musl-tools curl \
    && rm -rf /var/lib/apt/lists/*

# 4. 安装 Rust（确保 cargo 可用）
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
RUN ln -s /usr/local/cargo/bin/cargo /usr/bin/cargo  # 确保 cargo 可执行

# 5. 设置工作目录
WORKDIR /app

# 6. 复制 Cargo.toml 和 Cargo.lock，并进行依赖预构建（利用缓存）
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN /usr/bin/cargo build --release || true  # 明确使用 cargo 命令路径

# 7. 复制项目源码，并编译
COPY . .
RUN /usr/bin/cargo build --release

# 8. 运行阶段，使用更小的基础镜像
FROM debian:bullseye-slim

# 9. 设置工作目录
WORKDIR /app

# 10. 复制编译好的二进制文件
COPY --from=builder /app/target/release/clewdr /usr/local/bin/clewdr

# 11. 运行程序
CMD ["clewdr"]