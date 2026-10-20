# SPDX-FileCopyrightText: Copyright (c) 2026 NVIDIA CORPORATION & AFFILIATES. All rights reserved.
# SPDX-License-Identifier: Apache-2.0
#
# Licensed under the Apache License, Version 2.0 (the "License");
# you may not use this file except in compliance with the License.
# You may obtain a copy of the License at
#
#     http://www.apache.org/licenses/LICENSE-2.0
#
# Unless required by applicable law or agreed to in writing, software
# distributed under the License is distributed on an "AS IS" BASIS,
# WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
# See the License for the specific language governing permissions and
# limitations under the License.

# Base image of the serve stage. The default, "public-nginx", is the stage below:
# the public nginxinc/nginx-unprivileged image arranged to behave like NVIDIA's
# simple-nginx image. NVIDIA's CI passes SERVE_IMAGE=<simple-nginx image> and
# the public-nginx stage is then not built.
ARG SERVE_IMAGE=public-nginx

FROM node:22-alpine AS build

# Install project dependencies
WORKDIR /app
COPY package.json .
COPY package-lock.json .

RUN --mount=type=secret,id=npmrc,target=/root/.npmrc npm install --ignore-scripts

# Build the project
COPY public public
COPY src src
COPY index.html .
COPY tsconfig.json .
COPY tsconfig.app.json .
COPY tsconfig.node.json .
COPY vite.config.ts .
RUN npm run build

# Public serve base laid out like simple-nginx, so the Helm chart works unchanged:
# - nginx reads /usr/local/nginx/conf/nginx.conf, where the chart mounts its config,
#   and /usr/local/nginx/conf/mime.types, which the chart config includes;
# - nginx runs as the non-root user "nvs" (uid/gid 1000), which owns the paths
#   nginx writes at runtime (pid file in /run, temp files in /var/cache/nginx);
# - nginx starts directly in the foreground, without the base image's entrypoint
#   scripts, and logs to stdout/stderr.
FROM docker.io/nginxinc/nginx-unprivileged:1.31.7-alpine-slim AS public-nginx
USER root
RUN addgroup -g 1000 nvs \
    && adduser -D -H -u 1000 -G nvs -h /nonexistent -s /sbin/nologin nvs \
    && mkdir -p /usr/local/nginx/conf /app \
    && mv /etc/nginx/nginx.conf /usr/local/nginx/conf/nginx.conf \
    && ln -s /usr/local/nginx/conf/nginx.conf /etc/nginx/nginx.conf \
    && cp /etc/nginx/mime.types /usr/local/nginx/conf/mime.types \
    && chown -R nvs:nvs /usr/local/nginx /var/cache/nginx \
    && chown nvs:nvs /run
WORKDIR /app
ENTRYPOINT []
CMD ["nginx", "-g", "daemon off;"]

# The serve stage serves /var/www/html with the nginx config provided by the
# Helm chart. Any nginx image that does so can be used: set SERVE_IMAGE to
# choose it (and SERVE_USER below to a non-root user it defines).
FROM ${SERVE_IMAGE} AS serve
COPY --from=build /app/dist /var/www/html
COPY ATTRIBUTIONS.md .

# Run nginx as an unprivileged user. "nvs" is defined by both the default base
# and simple-nginx; set SERVE_USER to match the user of another SERVE_IMAGE.
ARG SERVE_USER=nvs
USER ${SERVE_USER}

EXPOSE 8080

