import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'currencies.dart';
import 'daemon_api.dart';
import 'fiber_api.dart';
import 'models.dart';
import 'settings.dart';

const String _defaultFundingHex = '0xba43b7400';

class SettingsScreen extends StatefulWidget {
  const SettingsScreen({
    super.key,
    required this.settings,
    required this.daemon,
    required this.fiber,
  });

  final SettingsController settings;
  final DaemonApi daemon;
  final FiberApi fiber;

  @override
  State<SettingsScreen> createState() => _SettingsScreenState();
}

class _SettingsScreenState extends State<SettingsScreen> {
  late final TextEditingController _fiberRpc;
  late final TextEditingController _daemonUrl;
  late final TextEditingController _p2p;
  String? _error;
  String? _status;
  FiberChannel? _toTwine;
  TwineInfo? _twine;
  var _busy = false;

  UserSettings get _user => widget.settings.settings;

  @override
  void initState() {
    super.initState();
    _fiberRpc = TextEditingController(text: _user.fiberRpc);
    _daemonUrl = TextEditingController(text: _user.daemonUrl);
    _p2p = TextEditingController(text: _user.p2pAddress);
    widget.settings.addListener(_onSettings);
    _refresh();
  }

  @override
  void dispose() {
    widget.settings.removeListener(_onSettings);
    _fiberRpc.dispose();
    _daemonUrl.dispose();
    _p2p.dispose();
    super.dispose();
  }

  void _onSettings() {
    if (mounted) {
      setState(() {});
    }
  }

  Future<void> _persist({
    String? pubkey,
    bool? operatorTools,
    String? preferredCurrency,
  }) {
    return widget.settings.update(
      _user.copyWith(
        fiberRpc: _fiberRpc.text,
        daemonUrl: _daemonUrl.text,
        p2pAddress: _p2p.text,
        pubkey: pubkey,
        operatorTools: operatorTools,
        preferredCurrency: preferredCurrency,
      ),
    );
  }

  Future<void> _refresh() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await _persist();
      final pubkey = await widget.fiber.nodePubkey(_fiberRpc.text);
      await widget.settings.update(_user.copyWith(pubkey: pubkey));
      final twine = await widget.daemon.fetchTwine(_daemonUrl.text);
      FiberChannel? channel;
      if (twine.pubkey != null) {
        channel = await widget.fiber.channelTo(_fiberRpc.text, twine.pubkey!);
      }
      if (!mounted) {
        return;
      }
      setState(() {
        _twine = twine;
        _toTwine = channel;
        _status = 'This phone is ${shortPubkey(pubkey)}';
        _busy = false;
      });
    } catch (err) {
      if (!mounted) {
        return;
      }
      setState(() {
        _error = err.toString();
        _busy = false;
      });
    }
  }

  Future<void> _openTowardTwine() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await _persist();
      final twine = await widget.daemon.fetchTwine(_daemonUrl.text);
      final pubkey = twine.pubkey;
      if (pubkey == null) {
        throw DaemonException(twine.error ?? 'Twine pubkey unavailable');
      }
      await widget.fiber.connectPeer(
        _fiberRpc.text,
        pubkey: pubkey,
        address: twine.p2pAddress,
      );
      final existing = await widget.fiber.channelTo(_fiberRpc.text, pubkey);
      if (existing == null || !existing.open) {
        await widget.fiber.openChannel(
          _fiberRpc.text,
          pubkey: pubkey,
          fundingHex: _defaultFundingHex,
        );
      }
      final channel = await widget.fiber.channelTo(_fiberRpc.text, pubkey);
      if (!mounted) {
        return;
      }
      setState(() {
        _twine = twine;
        _toTwine = channel;
        _status = 'You can sell. Outbound channel to Twine is ready.';
        _busy = false;
      });
    } catch (err) {
      if (!mounted) {
        return;
      }
      setState(() {
        _error = err.toString();
        _busy = false;
      });
    }
  }

  Future<void> _askTwineChannel() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      await _persist();
      var pubkey = _user.pubkey;
      if (pubkey == null || pubkey.isEmpty) {
        pubkey = await widget.fiber.nodePubkey(_fiberRpc.text);
        await widget.settings.update(_user.copyWith(pubkey: pubkey));
      }
      final result = await widget.daemon.connect(
        _daemonUrl.text,
        pubkey: pubkey,
        address: _p2p.text,
      );
      if (!mounted) {
        return;
      }
      setState(() {
        _status = result.channelOpen
            ? 'You can receive CKB. Twine opened a return channel.'
            : result.message;
        _busy = false;
      });
    } catch (err) {
      if (!mounted) {
        return;
      }
      setState(() {
        _error = err.toString();
        _busy = false;
      });
    }
  }

  Future<void> _copy(String value) async {
    await Clipboard.setData(ClipboardData(text: value));
    if (!mounted) {
      return;
    }
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text('Copied')),
    );
  }

  String get _channelCopy {
    final channel = _toTwine;
    if (channel == null) {
      return 'No outbound channel to Twine yet. Open one to sell.';
    }
    if (channel.open) {
      return 'Outbound channel to Twine is ready. You can accept and lock a sell.';
    }
    return 'Outbound channel to Twine is ${channel.channelId ?? 'pending'}';
  }

  @override
  Widget build(BuildContext context) {
    final pubkey = _user.pubkey;
    final preferred = fiatByCode(_user.preferredCurrency);

    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          _SectionCard(
            title: 'You',
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (pubkey != null && pubkey.isNotEmpty) ...[
                  Text(
                    'Fiber pubkey',
                    style: Theme.of(context).textTheme.labelMedium?.copyWith(
                      color: Theme.of(context).colorScheme.onSurfaceVariant,
                    ),
                  ),
                  const SizedBox(height: 4),
                  SelectableText(
                    pubkey,
                    key: const Key('settings-pubkey'),
                  ),
                  Align(
                    alignment: Alignment.centerRight,
                    child: IconButton(
                      key: const Key('copy-pubkey'),
                      onPressed: () => _copy(pubkey),
                      icon: const Icon(Icons.copy),
                      tooltip: 'Copy pubkey',
                    ),
                  ),
                ] else
                  const Text(
                    'Read this phone’s Fiber node. The pubkey is your id on ads and trades.',
                  ),
                FilledButton(
                  key: const Key('refresh-node'),
                  onPressed: _busy ? null : _refresh,
                  child: const Text('Read this phone'),
                ),
              ],
            ),
          ),
          const SizedBox(height: 12),
          _SectionCard(
            title: 'Listing',
            child: DropdownButtonFormField<FiatCurrency>(
              key: const Key('settings-currency'),
              initialValue: preferred,
              decoration: const InputDecoration(
                labelText: 'Default currency',
                helperText: 'Used when you post a sell offer',
              ),
              items: [
                for (final currency in fiatCurrencies)
                  DropdownMenuItem(
                    value: currency,
                    child: Text(currency.label),
                  ),
              ],
              onChanged: _busy
                  ? null
                  : (value) {
                      if (value == null) {
                        return;
                      }
                      _persist(preferredCurrency: value.code);
                    },
            ),
          ),
          const SizedBox(height: 12),
          _SectionCard(
            title: 'Channels',
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Text(
                  _channelCopy,
                  key: const Key('channel-status'),
                ),
                if (_twine?.pubkey != null) ...[
                  const SizedBox(height: 8),
                  Text(
                    'Twine ${shortPubkey(_twine!.pubkey!)}',
                    style: Theme.of(context).textTheme.bodySmall,
                  ),
                ],
                const SizedBox(height: 12),
                FilledButton(
                  key: const Key('open-channel-twine'),
                  onPressed: _busy ? null : _openTowardTwine,
                  child: const Text('Open channel to sell'),
                ),
                const SizedBox(height: 8),
                OutlinedButton(
                  key: const Key('ask-twine-channel'),
                  onPressed: _busy ? null : _askTwineChannel,
                  child: const Text('Ask Twine for a return channel'),
                ),
                const SizedBox(height: 8),
                Text(
                  'Sell needs an outbound channel. Buy needs Twine to be able to pay you.',
                  style: Theme.of(context).textTheme.bodySmall?.copyWith(
                    color: Theme.of(context).colorScheme.onSurfaceVariant,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 12),
          _SectionCard(
            title: 'Coordinator',
            child: SwitchListTile(
              key: const Key('operator-tools'),
              contentPadding: EdgeInsets.zero,
              title: const Text('Dispute tools'),
              subtitle: const Text(
                'Show Award buyer and Award seller after someone files a dispute. '
                'Normal trades release without this.',
              ),
              value: _user.operatorTools,
              onChanged: (value) => _persist(operatorTools: value),
            ),
          ),
          const SizedBox(height: 12),
          _SectionCard(
            title: 'This phone',
            child: Column(
              children: [
                TextField(
                  key: const Key('settings-fiber-rpc'),
                  controller: _fiberRpc,
                  decoration: const InputDecoration(
                    labelText: 'Fiber RPC',
                    helperText: 'This phone’s fnn',
                  ),
                  keyboardType: TextInputType.url,
                ),
                TextField(
                  key: const Key('settings-p2p'),
                  controller: _p2p,
                  decoration: const InputDecoration(
                    labelText: 'Fiber P2P address',
                  ),
                ),
                TextField(
                  key: const Key('settings-daemon'),
                  controller: _daemonUrl,
                  decoration: const InputDecoration(
                    labelText: 'Twine daemon',
                    helperText:
                        'iOS simulator: 127.0.0.1. Android emulator: 10.0.2.2',
                  ),
                  keyboardType: TextInputType.url,
                ),
              ],
            ),
          ),
          if (_status != null) ...[
            const SizedBox(height: 12),
            Text(_status!, key: const Key('settings-status')),
          ],
          if (_error != null) ...[
            const SizedBox(height: 8),
            Text(_error!, key: const Key('error-message')),
          ],
        ],
      ),
    );
  }
}

class _SectionCard extends StatelessWidget {
  const _SectionCard({required this.title, required this.child});

  final String title;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    return Card(
      child: Padding(
        padding: const EdgeInsets.fromLTRB(16, 14, 16, 16),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              title,
              style: Theme.of(context).textTheme.titleMedium?.copyWith(
                fontWeight: FontWeight.w600,
              ),
            ),
            const SizedBox(height: 12),
            child,
          ],
        ),
      ),
    );
  }
}

String shortPubkey(String pubkey) {
  final text = pubkey.trim();
  if (text.length <= 12) {
    return text;
  }
  return '${text.substring(0, 6)}…${text.substring(text.length - 4)}';
}
