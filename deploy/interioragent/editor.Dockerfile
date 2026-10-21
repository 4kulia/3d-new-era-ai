# The web editor as static files under /app/. The workflow builds the wasm and
# lays the files out in the build context; this image only serves them.
FROM nginxinc/nginx-unprivileged:1.27-alpine
COPY editor-site/ /usr/share/nginx/html/
