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
              Row(
                children: [
                  if (mine)
                    Text(
                      'YOUR OFFER',
                      style: TextStyle(
                        color: muted,
                        fontSize: 12,
                        letterSpacing: 0.6,
                      ),
                    )
                  else
                    const SizedBox.shrink(),
                  const Spacer(),
                  Text(
                    ad.currency,
                    style: TextStyle(
                      color: muted,
                      fontSize: 12,
                      letterSpacing: 0.6,
                    ),
                  ),
                  const SizedBox(width: 8),
                  Text(
                    'CKB',
                    style: TextStyle(
                      color: muted,
                      fontSize: 12,
                      letterSpacing: 0.6,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 10),
              Text(
                ad.price,
                style: const TextStyle(
                  fontSize: 28,
                  fontWeight: FontWeight.w600,
                  letterSpacing: -0.6,
                ),
              ),
              Text(
                'per CKB',
                style: TextStyle(color: muted, fontSize: 14),
              ),
              Text(
                'Available ${ad.available} CKB',
                style: TextStyle(color: muted, fontSize: 14),
              ),
              if (ad.min.isNotEmpty && ad.max.isNotEmpty) ...[
                const SizedBox(height: 4),
                Text(
                  'Limit ${ad.min}–${ad.max}',
                  style: TextStyle(color: muted, fontSize: 14),
                ),
              ],
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
                ],
              ),
            ],
          ),
        ),
      ),
    );
  }
}
