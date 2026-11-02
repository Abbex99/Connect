FROM caddy:2.11.7-alpine
COPY --chmod=0755 connect-server /usr/local/bin/connect-server
USER 10001:10001
WORKDIR /state
STOPSIGNAL SIGTERM
ENTRYPOINT ["/usr/local/bin/connect-server"]
