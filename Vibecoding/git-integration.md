Add the ability to integrate with git repositories.
User should be able to commit db objects to a git repository.
User should be able to push/pull from the git repository.
Pull should not update database objects, but should show the user what files have changed.
- if a change is detected, the application should show a diff of the changes.
- if a change is detected, the application should show a list of the files that have changed.
- if a change is detected, the application should generate a sql update script to apply the changes to the database. AI generated if needed.
