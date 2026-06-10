This project contains a implementation of a Gatekeeper, an Orchestrator, a server and a client.
## Team members:
- Bastien Gadoury
- Estevan Schmitt
- Grégory Toureille

## How to run the project

You need to have Docker Engine installed and running on your machine. Then, you can run the following command in the root directory of the project:
```bash
docker-compose up --build
```
Then the backend will be up and running.

![containers.png](docs/containers.png) *There should be one container for each part of the project*

You will need to run the client separately. You can do it by running the following command in the root directory of the project:
```bash
cd client
cargo run
```
You will be able to connect to a new user or to an existing user if the password is correct.

![login-screen.png](docs/login-screen.png)

Then you will be redirected to the broker, and the register to the spatial service so it can assign you to a server.

![login-successful.png](docs/login-successful.png)

In the gatekeeper's logs you will be able to see the login feedbacks (wrong passeword, new user created...)

![gatekeeper-login.png](docs/gatekeeper-login.png)

When connected you will be able to see the quadtree with margins, area of interest, entities and debug information in the client.

![connected.png](docs/connected.png) *Game view for the client*

You can observe the authority change in the up left corner or in the servers and quadtree logs.

Environment variables are configured in the .env file.