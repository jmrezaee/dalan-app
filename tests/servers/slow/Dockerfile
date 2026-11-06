# Forwards :2225 to the password server a, with an 800 ms delay on every packet leaving this container.
FROM alpine:3.20
RUN apk add --no-cache socat iproute2
COPY run.sh /run.sh
RUN chmod +x /run.sh
CMD ["/run.sh"]
