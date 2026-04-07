namespace Sqail.DbService.Configuration;

public class AppConfig
{
    public int Port { get; set; } = 5100;
    public string ConnectionsFile { get; set; } = "connections.json";
    public string MetadataFile { get; set; } = "metadata.json";
    /// <summary>Kept for one-time migration from appsettings into connections.json.</summary>
    public List<SavedConnection> SavedConnections { get; set; } = [];
    public JwtOptions Jwt { get; set; } = new();
}

public class JwtOptions
{
    /// <summary>HS256 signing secret. Must be at least 32 chars.</summary>
    public string Secret { get; set; } = "change-me-to-a-long-random-secret-32chars+";
    public string Issuer { get; set; } = "sqail-dbservice";
    public string Audience { get; set; } = "sqail";
    /// <summary>Pre-shared API key clients present to /api/auth/token to obtain a JWT.</summary>
    public string ApiKey { get; set; } = "change-me-api-key";
    public int TokenLifetimeMinutes { get; set; } = 1440;
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
