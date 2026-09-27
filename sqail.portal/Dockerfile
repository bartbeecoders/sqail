# Build context is the repository root (the changelog comes from
# ../CHANGELOG.md):  docker build -f sqail.portal/Dockerfile .
FROM node:22-alpine AS build
WORKDIR /app/sqail.portal
RUN corepack enable pnpm
COPY sqail.portal/package.json sqail.portal/pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY CHANGELOG.md /app/CHANGELOG.md
COPY sqail.portal/ ./
RUN pnpm build

FROM nginx:alpine
COPY --from=build /app/sqail.portal/dist /usr/share/nginx/html
COPY sqail.portal/nginx.conf /etc/nginx/conf.d/default.conf
EXPOSE 80
