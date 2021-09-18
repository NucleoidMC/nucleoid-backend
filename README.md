# nucleoid-backend

Backend service used for the Nucleoid Minecraft server.

## Running with docker

You can easily deploy the backend using [`docker-compose`](https://docs.docker.com/compose/) on a server using the provided `Dockerfile` and `docker-compose.yml`.

You can clone the repository and run `docker-compose up` to start up the required databases and the backend itself. This will use the config file in `config/config.json`, where you can then further configure the backend, including things like the Discord integration.
