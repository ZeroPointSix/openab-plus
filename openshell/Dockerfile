FROM ghcr.io/openabdev/openab-native:beta

USER root
RUN apt-get purge -y --auto-remove gh 2>/dev/null || true
RUN rm -f \
    /usr/bin/gh \
    /usr/local/bin/gh \
    /etc/apt/sources.list.d/github-cli.list \
    /usr/share/keyrings/githubcli-archive-keyring.gpg
RUN apt-get update && apt-get install -y --no-install-recommends iproute2 && rm -rf /var/lib/apt/lists/*
RUN groupadd -g 1000660000 sandbox && \
    useradd -u 1000660000 -g sandbox -m -d /sandbox -s /bin/bash sandbox && \
    mkdir -p \
      /sandbox/bin \
      /sandbox/.local/bin \
      /sandbox/.cache \
      /sandbox/tmp \
      /sandbox/work && \
    chown -R sandbox:sandbox /sandbox

USER sandbox
ENV HOME=/sandbox
ENV PATH=/sandbox/bin:/sandbox/.local/bin:/usr/local/bin:/usr/bin:/bin
ENV TMPDIR=/sandbox/tmp
WORKDIR /sandbox
