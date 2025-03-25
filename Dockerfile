# 基础镜像
FROM debian:bookworm-slim as builder

# 安装 Rust 工具链和构建依赖
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    curl \
    git \
    build-essential \
    cmake \
    perl \
    pkg-config \
    libclang-dev \
    musl-tools \
    && rm -rf /var/lib/apt/lists/*

# 安装 Rust
RUN curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /usr/src/app

# 创建一个新的空项目
RUN USER=root cargo new --bin clewdr
WORKDIR /usr/src/app/clewdr

# 复制 Cargo 配置文件
COPY Cargo.toml Cargo.lock* ./

# 创建必要的目录结构
RUN mkdir -p src/lib

# 创建一个虚拟的 lib.rs 和 main.rs 以缓存依赖项
RUN echo "// dummy file" > src/lib/lib.rs
RUN echo "fn main() {println!(\"Hello, world!\");}" > src/main.rs

# 构建依赖项
RUN cargo build --release

# 删除虚拟源文件
RUN rm -f src/lib/lib.rs src/main.rs

# 复制实际源代码
COPY src ./src

# 重新构建应用程序
RUN touch src/lib/lib.rs src/main.rs
RUN cargo build --release

# 最终镜像
FROM debian:bookworm-slim

# 安装运行时依赖
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /usr/local/bin

# 从构建器阶段复制二进制文件
COPY --from=builder /usr/src/app/clewdr/target/release/clewdr .

# 设置容器启动命令
CMD ["clewdr"]