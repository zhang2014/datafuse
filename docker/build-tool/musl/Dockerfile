ARG RUST_TOOLCHAIN
FROM datafuselabs/build-tool:base-${RUST_TOOLCHAIN}

ARG ARCH
ENV MUSL_TARGET ${ARCH}-linux-musl
ENV MUSL_RUST_TARGET ${ARCH}-unknown-linux-musl

# TODO:(everpcpc) change to tagged version after next release
# curl -sSfLo /tmp/musl-cross-make.tar.gz https://github.com/richfelker/musl-cross-make/archive/v0.9.9.tar.gz && \
#    tar -xzf /tmp/musl-cross-make.tar.gz -C /tmp
RUN curl -sSfLo /tmp/musl-cross-make.tar.gz https://github.com/richfelker/musl-cross-make/archive/fe915821b652a7fa37b34a596f47d8e20bc72338.zip && \
    unzip /tmp/musl-cross-make.tar.gz -d /tmp && \
    cd /tmp/musl-cross-make-* && \
    make install "-j$(nproc)" \
      OUTPUT=/usr/local/ \
      TARGET=${MUSL_TARGET} \
      DL_CMD='curl --retry 3 -sSfL -C - -o' && \
    rm -rf /tmp/musl-cross-make-* && \
    rm -f /tmp/musl-cross-make.tar.gz

ENV C_INCLUDE_PATH /usr/local/${MUSL_TARGET}/include/
RUN ln -s ${MUSL_TARGET}-gcc /usr/local/bin/musl-gcc

RUN rustup target add ${MUSL_RUST_TARGET}

# HACK: To avoid the `undefined reference to pthread_getname_np` error during linking,
# here we override set config environment var `je_cv_pthread_getname_np` to `no`,
# as suggested by tikv-jemalloc-sys's maintainer @BusyJay:
# https://github.com/tikv/jemallocator/issues/30#issuecomment-1183786410
#
# Hopefully this trick can be removed when rust upgraded to musl 1.2.3
# https://github.com/rust-lang/rust/pull/102891
ENV je_cv_pthread_getname_np no

# needed by z3
RUN ln -s ${MUSL_TARGET}-g++ /usr/local/bin/musl-g++
RUN ln -s ${MUSL_TARGET}-ar /usr/local/bin/musl-ar

# HACK: something wrong with python detection in cmake for z3
RUN ln -s python3.9 /usr/bin/python3.10

# HACK: to link with libstdc++ statically
# ref: https://github.com/rust-lang/rust/issues/36710#issuecomment-364623950
COPY linker.sh /usr/local/bin/linker
RUN chmod +x /usr/local/bin/linker
RUN printf "[target.${MUSL_RUST_TARGET}]\nlinker = \"linker\"\n" >${CARGO_HOME}/config
