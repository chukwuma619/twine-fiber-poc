import 'package:flutter/material.dart';

import 'amounts.dart';
import 'models.dart';

class TakeAdDialog extends StatefulWidget {
  const TakeAdDialog({super.key, required this.ad});

  final AdSnapshot ad;

  @override
  State<TakeAdDialog> createState() => _TakeAdDialogState();
}

class _TakeAdDialogState extends State<TakeAdDialog> {
  final TextEditingController _fiat = TextEditingController();
  String? _ckb;
  String? _error;

  @override
  void dispose() {
    _fiat.dispose();
    super.dispose();
  }

  void _recompute() {
    try {
      final ckb = ckbFromFiat(_fiat.text, widget.ad.rate);
      setState(() {
        _ckb = ckb;
        _error = null;
      });
    } catch (err) {
      setState(() {
        _ckb = null;
        _error = err.toString();
      });
    }
  }

  @override
  Widget build(BuildContext context) {
    return AlertDialog(
      title: Text('Buy from ${widget.ad.sellerName}'),
      content: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text('Rate ${widget.ad.rate} ${widget.ad.fiat} per CKB'),
          TextField(
            key: const Key('fiat-amount'),
            controller: _fiat,
            decoration: InputDecoration(
              labelText: 'Fiat amount (${widget.ad.fiat})',
            ),
            keyboardType: const TextInputType.numberWithOptions(decimal: true),
            onChanged: (_) => _recompute(),
          ),
          if (_ckb != null)
            Text('Locks $_ckb CKB', key: const Key('ckb-preview')),
          if (_error != null) Text(_error!),
        ],
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        FilledButton(
          key: const Key('start-trade'),
          onPressed: _ckb == null
              ? null
              : () => Navigator.of(context).pop(_fiat.text.trim()),
          child: const Text('Start trade'),
        ),
      ],
    );
  }
}
