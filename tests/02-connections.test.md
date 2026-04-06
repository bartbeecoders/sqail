# Connection Management

## Test: open new connection form

**Preconditions:** main UI is visible, sidebar is expanded

**Steps:**
1. Click the "+" (plus) button in the sidebar connections area to add a new connection

**Expected:**
- A connection form dialog/panel appears
- It contains fields for: name, driver, host, port, database, username, password
- A driver dropdown is present with options (PostgreSQL, MySQL, SQLite, MSSQL)

## Test: driver selection changes default port

**Preconditions:** connection form is open

**Steps:**
1. Select "PostgreSQL" from the driver dropdown
2. Note the port field value
3. Select "MySQL" from the driver dropdown
4. Note the port field value
5. Select "MSSQL" from the driver dropdown
6. Note the port field value

**Expected:**
- PostgreSQL sets port to 5432
- MySQL sets port to 3306
- MSSQL sets port to 1433

## Test: connection string mode toggle

**Preconditions:** connection form is open

**Steps:**
1. Fill in the host field with "myhost.example.com"
2. Fill in the database field with "testdb"
3. Find and click the connection string mode toggle button
4. Observe the connection string text area

**Expected:**
- A text area appears showing a connection string that includes "myhost.example.com" and "testdb"
- The individual form fields are hidden or replaced by the connection string input

## Test: close connection form without saving

**Preconditions:** connection form is open

**Steps:**
1. Click the close (X) button on the connection form

**Expected:**
- The connection form closes
- No new connection appears in the sidebar

## Test: save a new connection

**Preconditions:** connection form is open

**Steps:**
1. Type "Test Connection" in the name field
2. Select "PostgreSQL" from the driver dropdown
3. Type "localhost" in the host field
4. Leave port as default (5432)
5. Type "testdb" in the database field
6. Type "postgres" in the username field
7. Type "password123" in the password field
8. Click the Save button

**Expected:**
- The connection form closes
- A connection named "Test Connection" appears in the sidebar connections list
- It shows "PG" as the driver indicator

## Test: edit an existing connection

**Preconditions:** "Test Connection" exists in the sidebar

**Steps:**
1. Right-click (or find the edit action for) "Test Connection" in the sidebar
2. Click the edit/pencil option
3. Change the name to "Test Connection Edited"
4. Click Save

**Expected:**
- The connection form opens pre-filled with the connection details
- After saving, the sidebar shows "Test Connection Edited" instead of "Test Connection"

## Test: delete a connection

**Preconditions:** a test connection exists in the sidebar

**Steps:**
1. Right-click (or find the delete action for) the test connection in the sidebar
2. Click the delete/trash option
3. Confirm deletion if prompted

**Expected:**
- The connection is removed from the sidebar connections list
