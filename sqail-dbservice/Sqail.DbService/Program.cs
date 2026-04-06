using System.Text.Json;
using FastEndpoints;
using Sqail.DbService.Configuration;
using Sqail.DbService.Services;

var configPath = Path.Combine(AppContext.BaseDirectory, "appsettings.json");
if (!File.Exists(configPath))
{
    configPath = "appsettings.json";
}

var appConfig = new AppConfig();
if (File.Exists(configPath))
{
    var json = await File.ReadAllTextAsync(configPath);
    var parsed = JsonSerializer.Deserialize<AppConfig>(json, new JsonSerializerOptions
    {
        PropertyNameCaseInsensitive = true
    });
    if (parsed is not null)
        appConfig = parsed;
}

// One-time migration: if connections.json doesn't exist yet but appsettings has savedConnections, seed from it.
var connectionStore = new ConnectionStore(appConfig.ConnectionsFile);
if (!File.Exists(appConfig.ConnectionsFile) && appConfig.SavedConnections.Count > 0)
{
    foreach (var sc in appConfig.SavedConnections)
        await connectionStore.AddAsync(sc);
    Console.WriteLine($"Migrated {appConfig.SavedConnections.Count} connection(s) from appsettings.json -> {appConfig.ConnectionsFile}");
}

var builder = WebApplication.CreateBuilder(args);

builder.WebHost.UseUrls($"http://0.0.0.0:{appConfig.Port}");

builder.Services.AddSingleton(appConfig);
builder.Services.AddSingleton<IConnectionStore>(connectionStore);
builder.Services.AddSingleton<IConnectionManager, ConnectionManager>();
builder.Services.AddSingleton<IQueryService, QueryService>();
builder.Services.AddSingleton<IMetadataService, MetadataService>();
builder.Services.AddFastEndpoints();

builder.Services.AddCors(options =>
{
    options.AddDefaultPolicy(policy =>
    {
        policy.AllowAnyOrigin()
              .AllowAnyMethod()
              .AllowAnyHeader();
    });
});

var app = builder.Build();

app.UseCors();
app.UseFastEndpoints(c =>
{
    c.Serializer.Options.PropertyNamingPolicy = JsonNamingPolicy.CamelCase;
});

Console.WriteLine($"Sqail DB Service running on http://0.0.0.0:{appConfig.Port}");
Console.WriteLine($"Connections file : {appConfig.ConnectionsFile}");
Console.WriteLine($"Metadata file    : {appConfig.MetadataFile}");
Console.WriteLine("Endpoints:");
Console.WriteLine("  GET    /api/saved-connections          - List saved connections");
Console.WriteLine("  POST   /api/saved-connections          - Add a saved connection");
Console.WriteLine("  PUT    /api/saved-connections/{id}     - Update a saved connection");
Console.WriteLine("  DELETE /api/saved-connections/{id}     - Delete a saved connection");
Console.WriteLine("  POST   /api/connections/connect        - Connect to a database");
Console.WriteLine("  POST   /api/connections/disconnect     - Disconnect");
Console.WriteLine("  POST   /api/connections/test           - Test a connection");
Console.WriteLine("  GET    /api/connections                - List active connections");
Console.WriteLine("  POST   /api/query/execute              - Execute SQL query");
Console.WriteLine("  POST   /api/sproc/execute              - Execute stored procedure");
Console.WriteLine("  POST   /api/metadata                   - Get full database metadata");
Console.WriteLine("  GET    /api/metadata/{id}/tables       - Get tables");
Console.WriteLine("  GET    /api/metadata/{id}/views        - Get views");
Console.WriteLine("  GET    /api/metadata/{id}/procedures   - Get stored procedures");
Console.WriteLine("  GET    /api/metadata/{id}/functions    - Get functions");

app.Run();
