# 後端映像。build context 必須是 repo 根目錄（migrations 以 sqlx::migrate! 編譯期內嵌，路徑 ../../db/migrations）：
#   docker build -f apps/api/Dockerfile -t wishly-api .
# 環境變數見 apps/api/.env.example；未設 APP_ENV 即 production（缺機密會拒絕啟動），容器內需設 BIND=0.0.0.0:8080。
FROM rust:1.99.0-bookworm AS build
WORKDIR /app/apps/api
# 依賴層：只複製 manifest，用空殼編譯一次，之後改 src 不必重編依賴。
COPY apps/api/Cargo.toml apps/api/Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && echo '' > src/lib.rs \
 && cargo build --release --locked --bin wishly-api \
 && rm -rf src target/release/wishly-api target/release/deps/wishly_api* target/release/.fingerprint/wishly-api*
COPY db/migrations /app/db/migrations
COPY apps/api/src ./src
RUN cargo build --release --locked --bin wishly-api

# distroless/cc：glibc 與 CA 憑證，無 shell、非 root。探針由 Dokploy／外部監控打 /healthz、/readyz，不用 HEALTHCHECK（映像無 curl）。
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=build /app/apps/api/target/release/wishly-api /wishly-api
ENV RUST_LOG=info
EXPOSE 8080
ENTRYPOINT ["/wishly-api"]
