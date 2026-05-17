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
Then the backend (gatekeeper, redis, orchestrator and potential servers) will be up and running.

You will need to run the client separately. You can do it by running the following command in the root directory of the project:
```bash
cd client
cargo run
```