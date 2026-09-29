# Docker 中心

镜像包含中心服务和 Web 管理界面，不包含节点桌面程序。默认通过 HTTP 监听 8787，数据保存在 `/data`；管理员令牌由运行环境传入，不写入镜像。

中心按“上游 + 文件路径”保留最新及上一版，成功发布后清理更旧的无引用内容，并每 60 秒补扫；下游仍只接收最新版。待发布上传最多保护 24 小时，在途下载结束前也不会被删除，因此暂时占用可能超过两版。旧库升级保留现有最新版，从下一次更新开始保留两版；未关联的旧内容在保护期结束后回收。

## 启动

首次生成令牌并保存在安全位置；之后启动使用同一个令牌。Web 登录使用该令牌。先在项目目录构建镜像：

```sh
export FILESYNC_ADMIN_TOKEN="$(openssl rand -hex 32)"
docker build --platform linux/amd64 -t filesync:0.1.0 .
docker run -d --name filesync-hub \
  --restart unless-stopped \
  -p 8787:8787 \
  -e FILESYNC_ADMIN_TOKEN \
  -v filesync-hub-data:/data \
  filesync:0.1.0
```

浏览器和节点填写 `http://服务器IP:8787`。已有中心占用 8787 时，先选其他宿主端口，例如 `-p 8788:8787`。HTTP 下令牌与内容明文传输。

也可在项目目录设置上述环境变量后运行 `docker compose up -d --build`。

```sh
docker logs -f filesync-hub
docker stop filesync-hub
docker start filesync-hub
```

删除或替换容器时保留数据卷；不要执行删除数据卷的命令。镜像升级不会自动迁移本机 `.local/hub` 的旧数据；默认是独立的新中心。

## 更新已有容器

如果按上方命令部署，先在当前终端设置**原来的** `FILESYNC_ADMIN_TOKEN`，再执行：

```sh
docker build --platform linux/amd64 -t filesync:0.1.0 .
docker stop filesync-hub
docker rm filesync-hub
docker run -d --name filesync-hub \
  --restart unless-stopped -p 8787:8787 \
  -e FILESYNC_ADMIN_TOKEN \
  -v filesync-hub-data:/data \
  filesync:0.1.0
```

这里复用原数据卷，节点和分发关系会保留。如果之前使用了其他端口、卷名、绑定目录或自定义配置，更新时沿用原设置。仅 `docker restart` 不会切换到新镜像。同一数据目录只允许一个中心运行。

镜像使用 UID/GID `10001:10001`。使用宿主目录挂载代替命名卷时，需给该用户读写权限。自定义配置可只读挂载到 `/etc/filesync/hub.toml`，数据目录保持 `/data`。

## 构建和推送

```sh
docker build --platform linux/amd64 -t filesync:0.1.0 .
```

如需推送到自己的仓库，给本地镜像添加仓库标签后执行 `docker push`。本次构建目标为 Linux amd64。镜像使用锁定的 npm/Cargo 依赖，从源码构建 Web 和 Rust 程序。

如果本机能联网，但容器无法下载依赖，可先加载 `rust:1.98.1-bookworm` 和 `debian:bookworm-slim` 基础镜像，然后执行 `sh scripts/build-docker-offline.sh`。该脚本在本机构建 Web、准备锁定的 Rust 依赖，再在无网络容器中编译 Linux 程序；需要本机 Node、Python 3 和 Rust 1.98.1。

构建后可运行 `python3 scripts/docker-smoke.py`，检查 HTTP、Web 资源、鉴权、文件上传下载、非 root 运行、正常停止，以及重建容器后的数据保留。验证使用临时容器和卷，不访问现有中心数据。
