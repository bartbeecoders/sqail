namespace Sqail.DbService.Configuration;

public class AppConfig
{
    public int Port { get; set; } = 5100;
    public string ConnectionsFile { get; set; } = "connections.json";
    public string MetadataFile { get; set; } = "metadata.json";
    /// <summary>Kept for one-time migration from appsettings into connections.json.</summary>
    public List<SavedConnection> SavedConnections { get; set; } = [];
}

public class SavedConnection
{
    public required string Id { get; set; }
    public required string Name { get; set; }
    public required string Provider { get; set; }
    public required string ConnectionString { get; set; }
    public int CommandTimeoutSeconds { get; set; } = 30;
    public bool AutoConnect { get; set; }
}
