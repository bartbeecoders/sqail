I want to create an improved version of Sqail (in The Sqail2 folder). With following base improvements:

- Sqail2 needs to be a fast, easy to use, but powerfull SQL Database editor and query tool
- All interactions with the database need to go through a sqail-service that handles the connection to the SQL server but is a full https rest endpoint.
    - Write this rest backend in Rust, highly efficient and secure.
- Needs to be able to handle different sql server types (MS SQL server, postgres, sqlite and other in the future)
- The sqail2 editor UI needs to be modern and fast. Create it fully in Rust.
- Needs to run on Windows and Linux (omarchy)

Create a complete plan, put this in an html document with steps that we can follow and mark as complete.

After the completion of each step, make sure the system builds and runs. Provide simple scripts that I can use to run the system.
Setup, as part of you test cycles, podman container, containing test databases (ms sql, postgress etc) This should be part of the project.


Ok, we have a first base version.

Create now for the services and the UI a script that packages it all for window. Add clear instructions on how to setup the database service (how to connnect this service to an actual ms sql server for instance). etc