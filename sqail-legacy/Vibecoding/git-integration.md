Add the ability to integrate with git repositories.
User should be able to commit db objects to a git repository.
User should be able to push/pull from the git repository.
Pull should not update database objects, but should show the user what files have changed.
- if a change is detected, the application should show a diff of the changes.
- if a change is detected, the application should show a list of the files that have changed.
- if a change is detected, the application should generate a sql update script to apply the changes to the database. AI generated if needed.

Add the possibility to connect to multiple git repos if needed. A git connection should be linked to a database connection.

Do this change in a separate branch.

Add some more features that are typically needed for a git integration:
- Branch management (create, switch, delete branches)
- Merge and rebase operations
- Conflict resolution interface
- Staging area for selective commits
- Commit history visualization
- Tag management
- Remote repository configuration
- Fetch, pull, and push with conflict detection
- Blame/annotation view for files
- Compare commits/diffs in a visual interface
