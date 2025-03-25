# 1. 基础构建阶段，使用 Rust 官方镜像
FROM rustlang/rust:nightly as builder

# 2. 设置工作目录
WORKDIR /app

# 3. 复制 Cargo.toml 和 Cargo.lock 以利用 Docker 缓存
COPY Cargo.toml Cargo.lock ./

# 4. 创建一个虚拟的 src 目录，并进行依赖预构建（利用缓存加速构建）
RUN mkdir src && echo 'fn main() {}' > src/main.rs
RUN cargo build --release || true

# 5. 复制源代码
COPY . .

# 6. 重新编译正式版本
RUN cargo build --release

# 7. 运行阶段，使用更小的镜像（Debian Slim 或 Distroless）
FROM debian:bullseye-slim

# 8. 设置工作目录
WORKDIR /app

# 9. 复制编译好的二进制文件
COPY --from=builder /app/target/release/clewdr /usr/local/bin/clewdr

# 10. 运行程序
CMD ["clewdr"]