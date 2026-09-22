import 'package:flutter/material.dart';

import 'daemon_api.dart';
import 'fiber_api.dart';
import 'settings.dart';

class PostAdScreen extends StatefulWidget {
  const PostAdScreen({
    super.key,
    required this.settings,
    required this.daemon,
    required this.fiber,
  });

  final SettingsController settings;
  final DaemonApi daemon;
  final FiberApi fiber;

  @override
  State<PostAdScreen> createState() => _PostAdScreenState();
}

class _PostAdScreenState extends State<PostAdScreen> {
  final TextEditingController _amount = TextEditingController();
  final TextEditingController _fiat = TextEditingController(text: 'NGN');
  final TextEditingController _rate = TextEditingController();
  final TextEditingController _method = TextEditingController(text: 'Opay');
  String? _error;
  var _busy = false;

  UserSettings get _user => widget.settings.settings;

  Future<void> _submit() async {
    setState(() {
      _busy = true;
      _error = null;
    });
    try {
      var pubkey = _user.pubkey;
      if (pubkey == null || pubkey.isEmpty) {
        pubkey = await widget.fiber.nodePubkey(_user.fiberRpc);
        await widget.settings.update(_user.copyWith(pubkey: pubkey));
      }
      await widget.daemon.createAd(
        _user.daemonUrl,
        sellerPubkey: pubkey,
        sellerName: _user.name.trim().isEmpty ? 'Seller' : _user.name.trim(),
        availableCkb: _amount.text,
        fiat: _fiat.text,
        rate: _rate.text,
        paymentMethod: _method.text,
      );
      if (!mounted) {
        return;
      }
      Navigator.of(context).pop();
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
  void dispose() {
    _amount.dispose();
    _fiat.dispose();
    _rate.dispose();
    _method.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(title: const Text('New sell offer')),
      body: ListView(
        padding: const EdgeInsets.all(16),
        children: [
          TextField(
            key: const Key('ad-amount'),
            controller: _amount,
            decoration: const InputDecoration(labelText: 'CKB for sale'),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
          ),
          TextField(
            key: const Key('ad-fiat'),
            controller: _fiat,
            decoration: const InputDecoration(labelText: 'Fiat label'),
          ),
          TextField(
            key: const Key('ad-rate'),
            controller: _rate,
            decoration: const InputDecoration(labelText: 'Rate (fiat per 1 CKB)'),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
          ),
          TextField(
            key: const Key('ad-method'),
            controller: _method,
            decoration: const InputDecoration(labelText: 'Payment method'),
          ),
          const SizedBox(height: 16),
          FilledButton(
            key: const Key('post-ad'),
            onPressed: _busy ? null : _submit,
            child: Text(_busy ? 'Working...' : 'Post ad'),
          ),
          if (_error != null) ...[
            const SizedBox(height: 12),
            Text(_error!, key: const Key('error-message')),
          ],
        ],
      ),
    );
  }
}
