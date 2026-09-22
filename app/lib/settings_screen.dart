import 'package:flutter/material.dart';

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
  late final TextEditingController _name;
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
    _name = TextEditingController(text: _user.name);
    _fiberRpc = TextEditingController(text: _user.fiberRpc);
    _daemonUrl = TextEditingController(text: _user.daemonUrl);
    _p2p = TextEditingController(text: _user.p2pAddress);
    _refresh();
  }

  @override
  void dispose() {
    _name.dispose();
    _fiberRpc.dispose();
    _daemonUrl.dispose();
    _p2p.dispose();
    super.dispose();
  }

  Future<void> _persist({String? pubkey, bool? operatorTools}) {
    return widget.settings.update(
      _user.copyWith(
        name: _name.text,
        fiberRpc: _fiberRpc.text,
        daemonUrl: _daemonUrl.text,
        p2pAddress: _p2p.text,
        pubkey: pubkey,
        operatorTools: operatorTools,
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
        _status = 'Node pubkey $pubkey';
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
        _status = 'Opened your outbound channel to Twine. You can sell.';
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
        _status = result.message;
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

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('Settings')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          TextField(
            key: const Key('settings-name'),
            controller: _name,
            decoration: const InputDecoration(labelText: 'Display name'),
          ),
          TextField(
            key: const Key('settings-fiber-rpc'),
            controller: _fiberRpc,
            decoration: const InputDecoration(labelText: 'Your Fiber RPC'),
            keyboardType: TextInputType.url,
          ),
          TextField(
            key: const Key('settings-p2p'),
            controller: _p2p,
            decoration: const InputDecoration(labelText: 'Your Fiber P2P address'),
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
          SwitchListTile(
            key: const Key('operator-tools'),
            title: const Text('Operator tools'),
            subtitle: const Text('Show award buttons on disputed trades'),
            value: _user.operatorTools,
            onChanged: (value) => _persist(operatorTools: value),
          ),
          const SizedBox(height: 8),
          if (_user.pubkey != null) Text('Pubkey: ${_user.pubkey}'),
          if (_twine?.pubkey != null) Text('Twine: ${_twine!.pubkey}'),
          if (_toTwine != null)
            Text(
              _toTwine!.open
                  ? 'Outbound channel to Twine is ready'
                  : 'Outbound channel to Twine is ${_toTwine!.channelId ?? "pending"}',
              key: const Key('channel-status'),
            )
          else
            const Text(
              'No outbound channel to Twine yet. Open one to sell.',
              key: Key('channel-status'),
            ),
          const SizedBox(height: 12),
          FilledButton(
            key: const Key('refresh-node'),
            onPressed: _busy ? null : _refresh,
            child: const Text('Read node info'),
          ),
          const SizedBox(height: 8),
          FilledButton(
            key: const Key('open-channel-twine'),
            onPressed: _busy ? null : _openTowardTwine,
            child: const Text('Open channel to Twine'),
          ),
          const SizedBox(height: 8),
          FilledButton(
            key: const Key('ask-twine-channel'),
            onPressed: _busy ? null : _askTwineChannel,
            child: const Text('Ask Twine to open a return channel'),
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
