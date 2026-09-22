import 'package:flutter/material.dart';

import 'models.dart';

class OfferCard extends StatelessWidget {
  const OfferCard({
    super.key,
    required this.ad,
    required this.mine,
    this.onTake,
  });

  final AdSnapshot ad;
  final bool mine;
  final VoidCallback? onTake;

  @override
  Widget build(BuildContext context) {
    final muted = Theme.of(context).colorScheme.onSurfaceVariant;
    return Card(
      key: Key('ad-${ad.id}'),
      child: InkWell(
        key: Key('take-${ad.id}'),
        onTap: onTake,
        borderRadius: BorderRadius.circular(12),
        child: Padding(
          padding: const EdgeInsets.fromLTRB(16, 14, 16, 16),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                mine ? 'YOUR OFFER' : 'SELLING',
                style: TextStyle(
                  color: muted,
                  fontSize: 12,
                  letterSpacing: 0.6,
                ),
              ),
              const SizedBox(height: 4),
              Text(
                ad.sellerName,
                style: TextStyle(color: muted, fontSize: 13),
              ),
              const SizedBox(height: 10),
              Text(
                '${ad.availableCkb} CKB',
                style: const TextStyle(
                  fontSize: 28,
                  fontWeight: FontWeight.w600,
                  letterSpacing: -0.6,
                ),
              ),
              Text(
                '${ad.rate} ${ad.fiat} / CKB',
                style: TextStyle(color: muted, fontSize: 14),
              ),
              const SizedBox(height: 14),
              Row(
                children: [
                  Icon(
                    Icons.account_balance_wallet_outlined,
                    size: 16,
                    color: muted,
                  ),
                  const SizedBox(width: 8),
                  Expanded(
                    child: Text(
                      ad.paymentMethod,
                      style: const TextStyle(fontSize: 14),
                    ),
                  ),
                  if (onTake != null)
                    Text(
                      'Buy',
                      style: TextStyle(
                        color: Theme.of(context).colorScheme.primary,
                        fontWeight: FontWeight.w600,
                      ),
                    ),
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}
