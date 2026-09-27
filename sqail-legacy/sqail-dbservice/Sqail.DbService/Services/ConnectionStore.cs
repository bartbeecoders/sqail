using System.Text.Json;
using Sqail.DbService.Configuration;

namespace Sqail.DbService.Services;

public interface IConnectionStore
{
    Task<List<SavedConnection>> GetAllAsync();
    Task AddAsync(SavedConnection connection);
    Task UpdateAsync(SavedConnection connection);
    Task DeleteAsync(string id);
}

public class ConnectionStore : IConnectionStore
{
    private static readonly JsonSerializerOptions JsonOptions = new()
    {
        WriteIndented = true,
        PropertyNamingPolicy = JsonNamingPolicy.CamelCase
    };

    private readonly string _filePath;
    private readonly SemaphoreSlim _lock = new(1, 1);

    public ConnectionStore(string filePath)
    {
        _filePath = filePath;
    }

    public async Task<List<SavedConnection>> GetAllAsync()
    {
        await _lock.WaitAsync();
        try { return await ReadAsync(); }
        finally { _lock.Release(); }
    }

    public async Task AddAsync(SavedConnection connection)
    {
        await _lock.WaitAsync();
        try
        {
            var list = await ReadAsync();
            if (list.Any(c => c.Id == connection.Id))
                throw new InvalidOperationException($"A connection with id '{connection.Id}' already exists.");
            list.Add(connection);
            await WriteAsync(list);
        }
        finally { _lock.Release(); }
    }

    public async Task UpdateAsync(SavedConnection connection)
    {
        await _lock.WaitAsync();
        try
        {
            var list = await ReadAsync();
            var idx = list.FindIndex(c => c.Id == connection.Id);
            if (idx < 0)
                throw new InvalidOperationException($"Connection '{connection.Id}' not found.");
            list[idx] = connection;
            await WriteAsync(list);
        }
        finally { _lock.Release(); }
    }

    public async Task DeleteAsync(string id)
    {
        await _lock.WaitAsync();
        try
        {
            var list = await ReadAsync();
            var removed = list.RemoveAll(c => c.Id == id);
            if (removed == 0)
                throw new InvalidOperationException($"Connection '{id}' not found.");
            await WriteAsync(list);
        }
        finally { _lock.Release(); }
    }

    private async Task<List<SavedConnection>> ReadAsync()
    {
        if (!File.Exists(_filePath))
            return [];
        var json = await File.ReadAllTextAsync(_filePath);
        return JsonSerializer.Deserialize<List<SavedConnection>>(json, JsonOptions) ?? [];
    }

    private async Task WriteAsync(List<SavedConnection> list)
    {
        var dir = Path.GetDirectoryName(_filePath);
        if (!string.IsNullOrEmpty(dir))
            Directory.CreateDirectory(dir);
        var json = JsonSerializer.Serialize(list, JsonOptions);
        await File.WriteAllTextAsync(_filePath, json);
    }
}
